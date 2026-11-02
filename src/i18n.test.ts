import { describe, expect, it } from "vitest";
import { isAnyTranslation, LANGUAGES, translate } from "./i18n";
import en from "./locales/en.json";

describe("translation", () => {
  it("fills named placeholders and leaves unknown ones", () => {
    expect(translate("en", "calibration_ms", { value: 42 })).toBe("42 ms");
    expect(translate("en", "calibration_ms")).toBe("{value} ms");
  });

  it("falls back to English, then to the code itself", () => {
    expect(translate("ru", "no_such_code_anywhere")).toBe("no_such_code_anywhere");
    expect(translate("ru", "start")).not.toBe(translate("en", "start"));
  });

  it("knows a default label in any language", () => {
    expect(isAnyTranslation(translate("ja", "node_app"), "node_app")).toBe(true);
    expect(isAnyTranslation("My player", "node_app")).toBe(false);
  });

  it("has every language's own name", () => {
    expect(LANGUAGES.map((l) => l.code)).toEqual(["en", "ru", "es", "zh", "ja"]);
    expect(Object.keys(en).length).toBeGreaterThan(100);
  });
});
