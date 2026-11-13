# Head-related filters

`hrtf.bin` holds the filters the engine places sound in space with. It is made from the **SADIE II database** (subject D1, the Neumann KU100 dummy head, 48 kHz, `D1_48K_24bit_256tap_FIR_SOFA.sofa`) by `make_hrtf.py`.

Copyright 2018, University of York. Licensed under the Apache License, Version 2.0: see [LICENSE](LICENSE). Database: https://www.york.ac.uk/sadie-project/database.html. Paper: C. Armstrong, L. Thresh, D. Murphy, G. Kearney, *A Perceptual Evaluation of Individual and Non-Individual HRTFs: A Case Study of the SADIE II Database*, Applied Sciences 8 (11), 2018, https://doi.org/10.3390/app8112029.

**Changes made to the data:** interpolated onto an even grid of about 5 degrees, cut to 128 taps, leveled so that a source in front plays as loud as the sound itself and no ear anywhere gets more than that (for sound with as much in every octave as music and speech about have), and stored as 16-bit integers.

**File format,** little-endian: `AGH1`, the number of directions (u16), taps per ear (u16), the value of one integer step (f32); then the directions as unit vectors (three i16 each: x to the right, y to the front, z up, 32767 is 1); then the filters (i16: the left ear's taps, then the right ear's, for each direction).
