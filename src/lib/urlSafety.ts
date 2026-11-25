const ASCII_CONTROL_RE = /[\u0000-\u001F\u007F]/;
const STRICT_HTTPS_PREFIX_RE = /^https:\/\/[^/\\]/i;

/**
 * Validate and normalize an untrusted external URL for safe `<a href>` rendering.
 *
 * Strictly permits `https://` URLs with a non-empty hostname and no embedded
 * userinfo/credentials (`user:pass@host`). Rejects `javascript:`, `data:`,
 * `file:`, `http:`, relative paths, triple-slash/backslash-normalized forms,
 * control characters, and malformed URLs.
 */
export function toSafeExternalHttpsUrl(
  raw: string | null | undefined,
): string | null {
  if (typeof raw !== "string") {
    return null;
  }

  const trimmed = raw.trim();
  if (
    !trimmed ||
    ASCII_CONTROL_RE.test(trimmed) ||
    !STRICT_HTTPS_PREFIX_RE.test(trimmed)
  ) {
    return null;
  }

  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return null;
  }

  if (parsed.protocol !== "https:") {
    return null;
  }

  if (!parsed.hostname) {
    return null;
  }

  if (parsed.username !== "" || parsed.password !== "") {
    return null;
  }

  // Also reject empty userinfo delimiters such as `https://@example.com` or `https://:@example.com`
  const authority = trimmed.slice("https://".length).split(/[/?#]/, 1)[0] ?? "";
  if (authority.includes("@") || authority.includes("\\")) {
    return null;
  }

  return parsed.toString();
}
