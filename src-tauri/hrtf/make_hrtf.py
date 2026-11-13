#!/usr/bin/env python3
"""Makes hrtf.bin, the head-related filters the engine places sound with, from a SOFA file of the
SADIE II database: D1_48K_24bit_256tap_FIR_SOFA.sofa, the KU100 dummy head (see README.md here).

The database measures rings of elevation up to 15 degrees apart. So the filters are interpolated
onto an even grid: without their delays, which are interpolated apart, otherwise the two sounds of
a delayed pair would comb. The engine then only takes the nearest direction.

Needs numpy, scipy and h5py. Usage: make_hrtf.py <file.sofa> <hrtf.bin>
"""
import struct
import sys

import h5py
import numpy as np
from scipy.signal import butter, resample, sosfiltfilt
from scipy.spatial import ConvexHull

RATE = 48000
TAPS = 128
# what comes before any sound in the measurements: the first taps are nothing but this wait
SKIP = 64
# the grid's step in degrees
STEP = 5
FFT = 512
# how the sound's start is found: in the low end, where a head changes it the least
ONSET_BELOW = 3000
ONSET_LEVEL = 0.2
OVERSAMPLE = 8


def directions(azimuth, elevation):
    """Unit vectors: x to the right, y to the front, z up. SOFA's azimuth goes to the left"""
    az, el = np.radians(azimuth), np.radians(elevation)
    return np.stack([-np.sin(az) * np.cos(el), np.cos(az) * np.cos(el), np.sin(el)], axis=-1)


def onsets(ir):
    """In samples, for each filter of (..., taps)"""
    low = sosfiltfilt(butter(4, ONSET_BELOW, fs=RATE, output="sos"), ir, axis=-1)
    fine = resample(low, ir.shape[-1] * OVERSAMPLE, axis=-1)
    loud = np.abs(fine) >= ONSET_LEVEL * np.abs(fine).max(axis=-1, keepdims=True)
    return loud.argmax(axis=-1) / OVERSAMPLE


def loudness(filters):
    """Each ear's power for sound with as much in every octave as in the next, as music and speech about have"""
    frequency = np.fft.rfftfreq(4096, 1 / RATE)
    weight = np.where((frequency >= 40) & (frequency <= 18000), 1 / np.maximum(frequency, 1), 0)
    power = np.abs(np.fft.rfft(filters, 4096, axis=-1)) ** 2
    return (power * weight).sum(axis=-1) / weight.sum()


def grid():
    """Directions about STEP degrees apart all over the sphere"""
    points = []
    for elevation in range(-90, 91, STEP):
        count = max(1, round(360 / STEP * np.cos(np.radians(elevation))))
        points += [(i * 360 / count, elevation) for i in range(count)]
    return np.array(points, dtype=float)


def main(source, target):
    with h5py.File(source, "r") as f:
        ir = f["Data.IR"][:].astype(np.float64)
        position = f["SourcePosition"][:]
        assert f["Data.SamplingRate"][0] == RATE
    measured = directions(position[:, 0], position[:, 1])

    delay = onsets(ir)
    frequency = np.fft.rfftfreq(FFT)
    spectrum = np.fft.rfft(ir, FFT, axis=-1)
    aligned = spectrum * np.exp(2j * np.pi * frequency * delay[..., None])

    # the triangle around each direction gives the measurements to mix and how much of each
    triangles = ConvexHull(measured).simplices
    inverse = np.linalg.inv(measured[triangles].transpose(0, 2, 1))

    wanted = grid()
    unit = directions(wanted[:, 0], wanted[:, 1])
    filters = np.empty((len(unit), 2, TAPS))
    for i, direction in enumerate(unit):
        weights = inverse @ direction
        best = weights.min(axis=1).argmax()
        mix = np.clip(weights[best], 0, None)
        mix /= mix.sum()
        around = triangles[best]
        together = np.tensordot(mix, aligned[around], axes=1)
        late = mix @ delay[around]
        full = np.fft.irfft(together * np.exp(-2j * np.pi * frequency * late[:, None]), FFT, axis=-1)
        filters[i] = full[:, SKIP : SKIP + TAPS]

    # a cut at the end would ring
    fade = TAPS // 8
    filters[..., -fade:] *= np.cos(np.linspace(0, np.pi / 2, fade)) ** 2

    # a source in front plays as loud as the sound itself: the volume set is what is heard. Elsewhere the
    # head may shade it, but no ear gets more than the sound itself either: loud music would clip
    front = np.argmax(unit @ np.array([0.0, 1.0, 0.0]))
    filters /= np.sqrt(loudness(filters)[front].max())
    filters /= np.sqrt(np.maximum(loudness(filters).max(axis=-1), 1))[:, None, None]

    scale = np.abs(filters).max() / 32767
    with open(target, "wb") as f:
        f.write(b"AGH1" + struct.pack("<HHf", len(unit), TAPS, scale))
        f.write(np.round(unit * 32767).astype("<i2").tobytes())
        f.write(np.round(filters / scale).astype("<i2").tobytes())

    ears = 10 * np.log10(loudness(filters))
    louder = ears.max(axis=-1)
    print(f"{len(unit)} directions; the louder ear from {louder.min():.1f} to {louder.max():.1f} dB")
    for name, vector in [("front", (0, 1, 0)), ("right", (1, 0, 0)), ("back", (0, -1, 0)), ("up", (0, 0, 1)), ("down", (0, 0, -1))]:
        i = np.argmax(unit @ np.array(vector))
        print(f"{name}: left {ears[i, 0]:.1f} dB, right {ears[i, 1]:.1f} dB")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
