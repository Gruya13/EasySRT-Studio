// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";
// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
import { normalizeUiLanguage } from "../i18nUtils.ts";

describe("UI language normalization defaults to English", () => {
  it("returns 'en' unless Serbian is explicitly selected", () => {
    assert.equal(normalizeUiLanguage(undefined), "en");
    assert.equal(normalizeUiLanguage(null), "en");
    assert.equal(normalizeUiLanguage(""), "en");
    assert.equal(normalizeUiLanguage("en"), "en");
    assert.equal(normalizeUiLanguage("EN-US"), "en");
    assert.equal(normalizeUiLanguage("de"), "en");
    assert.equal(normalizeUiLanguage("sr"), "sr");
    assert.equal(normalizeUiLanguage("SR-Latn"), "sr");
  });
});
