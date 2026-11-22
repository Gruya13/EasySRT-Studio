//! Deterministic Serbian Cyrillic <-> Latin script converter.
//!
//! Handles digraphs (`Љ`/`Lj`/`LJ`, `Њ`/`Nj`/`NJ`, `Џ`/`Dž`/`DŽ`) with smart casing,
//! prefix-boundary exceptions (`injekcija` -> `инјекција`, `konjugacija` -> `конјугација`,
//! `nadživeti` -> `надживети`, etc.), and preserves HTML/ASS tags, `[...]` markers,
//! URLs, and email addresses untouched.

use crate::models::SerbianScript;
use regex::Regex;
use std::sync::OnceLock;

/// Prefixes where `n+j` or `d+ž` cross a morphological boundary and must NOT
/// be merged into `њ` or `џ`.
const DIGRAPH_EXCEPTION_PREFIXES: &[&str] = &[
    "injekc",
    "injekt",
    "konjug",
    "konjunk",
    "adjekt",
    "nadživ",
    "nadziv",
    "odživ",
    "odziv",
    "podžanr",
    "podžup",
    "predživot",
    "predzivot",
    "tanjug",
];

fn protected_span_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:https?://[^\s<>]+|www\.[^\s<>]+|[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}|<[^>]+>|\{\\[^}]+\}|\[[^\]]+\])",
        )
        .expect("valid protected span regex")
    })
}

/// Converts text to the requested Serbian script (`Latin` or `Cyrillic`).
pub fn convert_script(text: &str, target: SerbianScript) -> String {
    match target {
        SerbianScript::Latin => to_latin(text),
        SerbianScript::Cyrillic => to_cyrillic(text),
    }
}

/// Converts Serbian Cyrillic text to Serbian Latin.
///
/// Smart-cases uppercase digraphs (`Љ`, `Њ`, `Џ`):
/// - `"ЉУБАВ"` -> `"LJUBAV"`
/// - `"Љубав"` -> `"Ljubav"`
/// - `"љубав"` -> `"ljubav"`
pub fn to_latin(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());

    for (idx, &ch) in chars.iter().enumerate() {
        let is_all_caps_context = || {
            let next_upper = chars
                .get(idx + 1)
                .map(|c| c.is_alphabetic() && c.is_uppercase())
                .unwrap_or(false);
            let next_alpha = chars
                .get(idx + 1)
                .map(|c| c.is_alphabetic())
                .unwrap_or(false);
            let prev_upper = idx > 0
                && chars
                    .get(idx - 1)
                    .map(|c| c.is_alphabetic() && c.is_uppercase())
                    .unwrap_or(false);
            next_upper || (!next_alpha && prev_upper)
        };

        match ch {
            'А' => out.push('A'),
            'Б' => out.push('B'),
            'В' => out.push('V'),
            'Г' => out.push('G'),
            'Д' => out.push('D'),
            'Ђ' => out.push('Đ'),
            'Е' => out.push('E'),
            'Ж' => out.push('Ž'),
            'З' => out.push('Z'),
            'И' => out.push('I'),
            'Ј' => out.push('J'),
            'К' => out.push('K'),
            'Л' => out.push('L'),
            'Љ' => {
                if is_all_caps_context() {
                    out.push_str("LJ");
                } else {
                    out.push_str("Lj");
                }
            }
            'М' => out.push('M'),
            'Н' => out.push('N'),
            'Њ' => {
                if is_all_caps_context() {
                    out.push_str("NJ");
                } else {
                    out.push_str("Nj");
                }
            }
            'О' => out.push('O'),
            'П' => out.push('P'),
            'Р' => out.push('R'),
            'С' => out.push('S'),
            'Т' => out.push('T'),
            'Ћ' => out.push('Ć'),
            'У' => out.push('U'),
            'Ф' => out.push('F'),
            'Х' => out.push('H'),
            'Ц' => out.push('C'),
            'Ч' => out.push('Č'),
            'Џ' => {
                if is_all_caps_context() {
                    out.push_str("DŽ");
                } else {
                    out.push_str("Dž");
                }
            }
            'Ш' => out.push('Š'),

            'а' => out.push('a'),
            'б' => out.push('b'),
            'в' => out.push('v'),
            'г' => out.push('g'),
            'д' => out.push('d'),
            'ђ' => out.push('đ'),
            'е' => out.push('e'),
            'ж' => out.push('ž'),
            'з' => out.push('z'),
            'и' => out.push('i'),
            'ј' => out.push('j'),
            'к' => out.push('k'),
            'л' => out.push('l'),
            'љ' => out.push_str("lj"),
            'м' => out.push('m'),
            'н' => out.push('n'),
            'њ' => out.push_str("nj"),
            'о' => out.push('o'),
            'п' => out.push('p'),
            'р' => out.push('r'),
            'с' => out.push('s'),
            'т' => out.push('t'),
            'ћ' => out.push('ć'),
            'у' => out.push('u'),
            'ф' => out.push('f'),
            'х' => out.push('h'),
            'ц' => out.push('c'),
            'ч' => out.push('č'),
            'џ' => out.push_str("dž"),
            'ш' => out.push('š'),

            other => out.push(other),
        }
    }

    out
}

/// Converts Serbian Latin text to Serbian Cyrillic, preserving protected spans
/// (URLs, emails, HTML/ASS tags, `[...]` markers) and respecting prefix-boundary
/// exceptions (`injekcija` -> `инјекција`, `nadživeti` -> `надживети`).
pub fn to_cyrillic(text: &str) -> String {
    let re = protected_span_regex();
    let mut out = String::with_capacity(text.len() * 2);
    let mut last_end = 0;

    for m in re.find_iter(text) {
        if m.start() > last_end {
            convert_plain_segment_to_cyrillic(&text[last_end..m.start()], &mut out);
        }
        out.push_str(m.as_str());
        last_end = m.end();
    }

    if last_end < text.len() {
        convert_plain_segment_to_cyrillic(&text[last_end..], &mut out);
    }

    out
}

fn convert_plain_segment_to_cyrillic(segment: &str, out: &mut String) {
    let mut word_buf = String::new();
    for ch in segment.chars() {
        if ch.is_alphabetic() {
            word_buf.push(ch);
        } else {
            if !word_buf.is_empty() {
                convert_word_to_cyrillic(&word_buf, out);
                word_buf.clear();
            }
            out.push(ch);
        }
    }
    if !word_buf.is_empty() {
        convert_word_to_cyrillic(&word_buf, out);
    }
}

fn is_digraph_exception_word(word_lower: &str) -> bool {
    DIGRAPH_EXCEPTION_PREFIXES
        .iter()
        .any(|prefix| word_lower.starts_with(prefix))
}

fn convert_word_to_cyrillic(word: &str, out: &mut String) {
    let lower = word.to_lowercase();
    let skip_digraph_merge = is_digraph_exception_word(&lower);
    let chars: Vec<char> = word.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();

        if !skip_digraph_merge {
            if let Some(n) = next {
                // Lj / LJ / lj
                if (ch == 'L' || ch == 'l') && (n == 'J' || n == 'j') {
                    if ch.is_uppercase() {
                        out.push('Љ');
                    } else {
                        out.push('љ');
                    }
                    i += 2;
                    continue;
                }
                // Nj / NJ / nj
                if (ch == 'N' || ch == 'n') && (n == 'J' || n == 'j') {
                    if ch.is_uppercase() {
                        out.push('Њ');
                    } else {
                        out.push('њ');
                    }
                    i += 2;
                    continue;
                }
                // Dž / DŽ / dž
                if (ch == 'D' || ch == 'd') && (n == 'Ž' || n == 'ž') {
                    if ch.is_uppercase() {
                        out.push('Џ');
                    } else {
                        out.push('џ');
                    }
                    i += 2;
                    continue;
                }
            }
        }

        // Note:even in exception words like "nadživeti" or "injekcija", "lj" is never an exception,
        // so if an exception word ever contained "lj", check that too:
        if skip_digraph_merge {
            if let Some(n) = next {
                if (ch == 'L' || ch == 'l') && (n == 'J' || n == 'j') {
                    if ch.is_uppercase() {
                        out.push('Љ');
                    } else {
                        out.push('љ');
                    }
                    i += 2;
                    continue;
                }
            }
        }

        out.push(map_single_latin_to_cyrillic(ch));
        i += 1;
    }
}

fn map_single_latin_to_cyrillic(ch: char) -> char {
    match ch {
        'A' => 'А',
        'B' => 'Б',
        'V' => 'В',
        'G' => 'Г',
        'D' => 'Д',
        'Đ' => 'Ђ',
        'E' => 'Е',
        'Ž' => 'Ж',
        'Z' => 'З',
        'I' => 'И',
        'J' => 'Ј',
        'K' => 'К',
        'L' => 'Л',
        'M' => 'М',
        'N' => 'Н',
        'O' => 'О',
        'P' => 'П',
        'R' => 'Р',
        'S' => 'С',
        'T' => 'Т',
        'Ć' => 'Ћ',
        'U' => 'У',
        'F' => 'Ф',
        'H' => 'Х',
        'C' => 'Ц',
        'Č' => 'Ч',
        'Š' => 'Ш',

        'a' => 'а',
        'b' => 'б',
        'v' => 'в',
        'g' => 'г',
        'd' => 'д',
        'đ' => 'ђ',
        'e' => 'е',
        'ž' => 'ж',
        'z' => 'з',
        'i' => 'и',
        'j' => 'ј',
        'k' => 'к',
        'l' => 'л',
        'm' => 'м',
        'n' => 'н',
        'o' => 'о',
        'p' => 'п',
        'r' => 'р',
        's' => 'с',
        't' => 'т',
        'ć' => 'ћ',
        'u' => 'у',
        'f' => 'ф',
        'h' => 'х',
        'c' => 'ц',
        'č' => 'ч',
        'š' => 'ш',

        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyrillic_to_latin_digraph_smart_casing() {
        assert_eq!(to_latin("ЉУБАВ"), "LJUBAV");
        assert_eq!(to_latin("Љубав"), "Ljubav");
        assert_eq!(to_latin("љубав"), "ljubav");

        assert_eq!(to_latin("ЊЕГОШ"), "NJEGOŠ");
        assert_eq!(to_latin("Његош"), "Njegoš");
        assert_eq!(to_latin("његош"), "njegoš");

        assert_eq!(to_latin("ЏЕП"), "DŽEP");
        assert_eq!(to_latin("Џеп"), "Džep");
        assert_eq!(to_latin("џеп"), "džep");

        assert_eq!(to_latin("КОЊ"), "KONJ");
        assert_eq!(to_latin("Коњ"), "Konj");
    }

    #[test]
    fn latin_to_cyrillic_digraphs_and_exceptions() {
        assert_eq!(to_cyrillic("Ljubav i pažnja u džepu."), "Љубав и пажња у џепу.");
        assert_eq!(to_cyrillic("LJUBAV I PAŽNJA U DŽEPU."), "ЉУБАВ И ПАЖЊА У ЏЕПУ.");

        // Prefix-boundary exceptions where n+j or d+ž stay separate
        assert_eq!(to_cyrillic("injekcija"), "инјекција");
        assert_eq!(to_cyrillic("Injekcija"), "Инјекција");
        assert_eq!(to_cyrillic("INJEKCIJA"), "ИНЈЕКЦИЈА");
        assert_eq!(to_cyrillic("konjugacija"), "конјугација");
        assert_eq!(to_cyrillic("konjunkcija"), "конјункција");
        assert_eq!(to_cyrillic("adjektiv"), "адјектив");
        assert_eq!(to_cyrillic("nadživeti"), "надживети");
        assert_eq!(to_cyrillic("Nadživela"), "Надживела");
        assert_eq!(to_cyrillic("odživeti"), "одживети");
        assert_eq!(to_cyrillic("podžanr"), "поджанр");
        assert_eq!(to_cyrillic("podžupan"), "поджупан");
        assert_eq!(to_cyrillic("predživot"), "предживот");
        assert_eq!(to_cyrillic("Tanjug"), "Танјуг");
    }

    #[test]
    fn preserves_tags_urls_emails_and_markers() {
        let input = "<i>Posetite https://example.com/ljubav ili pišite na info@primer.rs [SMEH] {\\an8}</i>";
        let cyr = to_cyrillic(input);
        assert_eq!(
            cyr,
            "<i>Посетите https://example.com/ljubav или пишите на info@primer.rs [SMEH] {\\an8}</i>"
        );
    }

    #[test]
    fn round_trip_standard_serbian_text() {
        let latin = "Đorđe Čvarkov i Žika Šarenica gledaju 12:30 film u Nišu.";
        let cyr = convert_script(latin, SerbianScript::Cyrillic);
        assert_eq!(
            cyr,
            "Ђорђе Чварков и Жика Шареница гледају 12:30 филм у Нишу."
        );
        let back_to_latin = convert_script(&cyr, SerbianScript::Latin);
        assert_eq!(back_to_latin, latin);
    }
}
