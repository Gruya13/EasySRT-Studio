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

const EXACT_IJEKAVIAN_TO_EKAVIAN: &[(&str, &str)] = &[
    ("ovdje", "ovde"),
    ("gdje", "gde"),
    ("igdje", "igde"),
    ("nigdje", "nigde"),
    ("negdje", "negde"),
    ("svugdje", "svugde"),
    ("prije", "pre"),
    ("poslije", "posle"),
    ("uvijek", "uvek"),
    ("zauvijek", "zauvek"),
    ("oduvijek", "oduvek"),
    ("vrijeme", "vreme"),
    ("čovjek", "čovek"),
    ("čovječe", "čoveče"),
    ("čovjeka", "čoveka"),
    ("čovjeku", "čoveku"),
    ("čovjekom", "čovekom"),
    ("dijete", "dete"),
    ("djeteta", "deteta"),
    ("djetetu", "detetu"),
    ("djetetom", "detetom"),
    ("djeca", "deca"),
    ("djece", "dece"),
    ("djeci", "deci"),
    ("djecu", "decu"),
    ("djecom", "decom"),
    ("djed", "deda"),
    ("djeda", "dede"),
    ("djedu", "dedi"),
    ("pradjed", "pradeda"),
    ("pradjedova", "pradedova"),
    ("pradjedovi", "pradedovi"),
    ("hljeb", "hleb"),
    ("hljeba", "hleba"),
    ("hljebu", "hlebu"),
    ("mlijeko", "mleko"),
    ("mlijeka", "mleka"),
    ("mlijeku", "mleku"),
    ("rijeka", "reka"),
    ("rijeke", "reke"),
    ("rijeci", "reci"),
    ("rijeku", "reku"),
    ("rijekom", "rekom"),
    ("riječ", "reč"),
    ("riječi", "reči"),
    ("riječima", "rečima"),
    ("vijest", "vest"),
    ("vijesti", "vesti"),
    ("povijest", "istorija"),
    ("svijet", "svet"),
    ("svijeta", "sveta"),
    ("svijetu", "svetu"),
    ("svijetom", "svetom"),
    ("svjetli", "svetli"),
    ("svjetla", "svetla"),
    ("svjetlo", "svetlo"),
    ("svjetlost", "svetlost"),
    ("cvijet", "cvet"),
    ("cvijeta", "cveta"),
    ("cvjetna", "cvetna"),
    ("cvjetni", "cvetni"),
    ("cvjetno", "cvetno"),
    ("ljeto", "leto"),
    ("ljeta", "leta"),
    ("ljetu", "letu"),
    ("vjetar", "vetar"),
    ("vjetra", "vetra"),
    ("vjetru", "vetru"),
    ("mjesto", "mesto"),
    ("mjesta", "mesta"),
    ("mjestu", "mestu"),
    ("mjestom", "mestom"),
    ("mjera", "mera"),
    ("mjere", "mere"),
    ("mjeru", "meru"),
    ("nedjelja", "nedelja"),
    ("nedjelje", "nedelje"),
    ("nedjelju", "nedelju"),
    ("ponedjeljak", "ponedeljak"),
    ("srijeda", "sreda"),
    ("srijedu", "sredu"),
    ("uspjeh", "uspeh"),
    ("uspjeha", "uspeha"),
    ("uspjehu", "uspehu"),
    ("uspješno", "uspešno"),
    ("uspješan", "uspešan"),
    ("uspješna", "uspešna"),
    ("osmijeh", "osmeh"),
    ("osmijeha", "osmeha"),
    ("smijeh", "smeh"),
    ("smijeha", "smeha"),
    ("grijeh", "greh"),
    ("grijeha", "greha"),
    ("lijep", "lep"),
    ("lijepa", "lepa"),
    ("lijepo", "lepo"),
    ("lijepog", "lepog"),
    ("lijepom", "lepom"),
    ("ljepota", "lepota"),
    ("bijel", "beo"),
    ("bijela", "bela"),
    ("bijelo", "belo"),
    ("bijeli", "beli"),
    ("cijeli", "ceo"),
    ("cijela", "cela"),
    ("cijelo", "celo"),
    ("cijelog", "celog"),
    ("cijelom", "celom"),
    ("dio", "deo"),
    ("dijela", "dela"),
    ("dijelu", "delu"),
    ("htjeti", "hteti"),
    ("vidjeti", "videti"),
    ("željeti", "želeti"),
    ("živjeti", "živeti"),
    ("voljeti", "voleti"),
    ("umrijeti", "umreti"),
    ("mrijeti", "mreti"),
    ("grijeti", "grejati"),
    ("grije", "greje"),
];

const STEM_IJEKAVIAN_TO_EKAVIAN: &[(&str, &str)] = &[
    ("zabilješk", "zabelešk"),
    ("zabiljež", "zabelež"),
    ("bilješk", "belešk"),
    ("nijemac", "nemac"),
    ("nijemci", "nemci"),
    ("njemačk", "nemačk"),
    ("zahtijev", "zahtev"),
    ("zahtjev", "zahtev"),
    ("primijet", "primet"),
    ("primjeć", "primeć"),
    ("razumijev", "razumev"),
    ("razumjet", "razumet"),
    ("povjerenj", "poverenj"),
    ("vjerov", "verov"),
    ("vjeruj", "veruj"),
    ("pjesm", "pesm"),
    ("djevoj", "devoj"),
    ("osjeć", "oseć"),
    ("osjet", "oset"),
    ("prebjeg", "prebeg"),
    ("pobjeg", "pobeg"),
    ("pobjed", "pobed"),
    ("bjež", "bež"),
    ("svjedoč", "svedoč"),
    ("svjedok", "svedok"),
    ("vrijedn", "vredn"),
    ("vrjedn", "vredn"),
];

fn is_cyrillic_alpha(ch: char) -> bool {
    matches!(
        ch,
        'А'..='Я' | 'а'..='я' | 'Ђ' | 'ђ' | 'Ј' | 'ј' | 'Љ' | 'љ' | 'Њ' | 'њ' | 'Ћ' | 'ћ' | 'Џ' | 'џ'
    )
}

fn apply_case_pattern(template_latin: &str, replacement_lower: &str) -> String {
    let alpha_chars: Vec<char> = template_latin
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect();
    if alpha_chars.len() > 1 && alpha_chars.iter().all(|c| c.is_uppercase()) {
        return replacement_lower.to_uppercase();
    }
    if template_latin
        .chars()
        .next()
        .is_some_and(|c| c.is_uppercase())
    {
        let mut chars = replacement_lower.chars();
        if let Some(first) = chars.next() {
            let mut out: String = first.to_uppercase().collect();
            out.push_str(chars.as_str());
            return out;
        }
    }
    replacement_lower.to_string()
}

fn normalize_ekavian_word(token: &str) -> String {
    let was_cyrillic = token.chars().any(is_cyrillic_alpha);
    let latin = if was_cyrillic {
        to_latin(token)
    } else {
        token.to_string()
    };
    let lower = latin.to_lowercase();

    let replaced_lower = if let Some(&(_, ekav)) = EXACT_IJEKAVIAN_TO_EKAVIAN
        .iter()
        .find(|&&(ijek, _)| ijek == lower)
    {
        Some(ekav.to_string())
    } else {
        STEM_IJEKAVIAN_TO_EKAVIAN
            .iter()
            .find_map(|&(ijek_stem, ekav_stem)| {
                lower
                    .strip_prefix(ijek_stem)
                    .map(|suffix| format!("{ekav_stem}{suffix}"))
            })
    };

    let Some(next_lower) = replaced_lower else {
        return token.to_string();
    };

    let cased_latin = apply_case_pattern(&latin, &next_lower);
    if was_cyrillic {
        to_cyrillic(&cased_latin)
    } else {
        cased_latin
    }
}

fn normalize_ekavian_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    let mut word_buf = String::new();

    for ch in segment.chars() {
        if ch.is_alphabetic() {
            word_buf.push(ch);
        } else {
            if !word_buf.is_empty() {
                out.push_str(&normalize_ekavian_word(&word_buf));
                word_buf.clear();
            }
            out.push(ch);
        }
    }

    if !word_buf.is_empty() {
        out.push_str(&normalize_ekavian_word(&word_buf));
    }

    out
}

/// Normalizes common unambiguous Ijekavian words and stems into Serbian Ekavian
/// while preserving casing, URLs, emails, HTML/ASS tags, and Cyrillic/Latin script.
pub fn normalize_ekavian(text: &str) -> String {
    let re = protected_span_regex();
    let mut out = String::with_capacity(text.len());
    let mut last_end = 0;

    for m in re.find_iter(text) {
        if m.start() > last_end {
            out.push_str(&normalize_ekavian_segment(&text[last_end..m.start()]));
        }
        out.push_str(m.as_str());
        last_end = m.end();
    }

    if last_end < text.len() {
        out.push_str(&normalize_ekavian_segment(&text[last_end..]));
    }

    out
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

    #[test]
    fn normalizes_ijekavian_to_ekavian_preserving_case_and_script() {
        let input = "Ostajte ovdje, sunce tuđeg neba, zahtjevao sam zabilješku od Nijemaca za djecu i pjesme.";
        assert_eq!(
            normalize_ekavian(input),
            "Ostajte ovde, sunce tuđeg neba, zahtevao sam zabelešku od Nemaca za decu i pesme."
        );

        let cyr_input = "Овдје је лијепо вријеме за дјецу и пјесме.";
        assert_eq!(
            normalize_ekavian(cyr_input),
            "Овде је лепо време за децу и песме."
        );

        assert_eq!(normalize_ekavian("OVDJE I UVIJEK"), "OVDE I UVEK");
    }
}
