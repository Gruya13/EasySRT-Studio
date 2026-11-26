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

/// Converts every non-empty string yielded by `values` in place to the target Serbian
/// script (`Latin` or `Cyrillic`). Keeps multi-style translation variant maps in sync
/// with the project's active script regardless of map key type.
pub fn convert_script_values<'a>(
    values: impl Iterator<Item = &'a mut String>,
    script: SerbianScript,
) {
    for value in values {
        if !value.is_empty() {
            *value = convert_script(value, script);
        }
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
    ("hljebom", "hlebom"),
    ("hljebovi", "hlebovi"),
    ("hljebova", "hlebova"),
    ("hljebove", "hlebove"),
    ("hljebovima", "hlebovima"),
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
    ("vidio", "video"),
    ("htio", "hteo"),
    ("želio", "želeo"),
    ("živio", "živeo"),
    ("volio", "voleo"),
    ("smio", "smeo"),
    ("razumio", "razumeo"),
    ("grijeti", "grejati"),
    ("grijati", "grejati"),
    ("grijao", "grejao"),
    ("grijala", "grejala"),
    ("grijali", "grejali"),
    ("grijalo", "grejalo"),
    ("grijem", "grejem"),
    ("griješ", "greješ"),
    ("grije", "greje"),
    ("grijemo", "grejemo"),
    ("grijete", "grejete"),
    ("griju", "greju"),
    ("grijanje", "grejanje"),
    ("zagrijati", "zagrejati"),
    ("ugrijati", "ugrejati"),
    ("ogrijati", "ogrejati"),
    ("navijek", "navek"),
    ("naprijed", "napred"),
    ("otprije", "otpre"),
    ("prijeko", "preko"),
    ("prijeki", "preki"),
    ("prijeka", "preka"),
    ("prijekog", "prekog"),
    ("djelo", "delo"),
    ("djela", "dela"),
    ("djelu", "delu"),
    ("djelom", "delom"),
    ("djelima", "delima"),
    ("tijelo", "telo"),
    ("tijela", "tela"),
    ("tijelu", "telu"),
    ("tijelom", "telom"),
    ("tijelima", "telima"),
    ("sjesti", "sesti"),
    ("sjedim", "sedim"),
    ("sjediš", "sediš"),
    ("sjedi", "sedi"),
    ("sjedimo", "sedimo"),
    ("sjedite", "sedite"),
    ("sjede", "sede"),
    ("sjedio", "sedeo"),
    ("sjedila", "sedela"),
    ("sjedili", "sedeli"),
    // Unambiguous Croatian pronouns and lexical forms -> standard Serbian
    ("tko", "ko"),
    ("netko", "neko"),
    ("nitko", "niko"),
    ("svatko", "svako"),
    ("itko", "iko"),
    ("tkogod", "kogod"),
    ("krajolik", "pejzaž"),
    ("krajolika", "pejzaža"),
    ("krajoliku", "pejzažu"),
    ("krajolikom", "pejzažem"),
    ("krajolici", "pejzaži"),
    ("krajolike", "pejzaže"),
    ("krajolicima", "pejzažima"),
    ("poviješću", "istorijom"),
    ("povijesni", "istorijski"),
    ("povijesna", "istorijska"),
    ("povijesno", "istorijsko"),
    ("povijesne", "istorijske"),
    ("povijesnu", "istorijsku"),
    ("povijesnog", "istorijskog"),
    ("povijesnoj", "istorijskoj"),
    ("povijesnom", "istorijskom"),
    ("povijesnih", "istorijskih"),
    ("povijesnim", "istorijskim"),
    // `tisuća` -> `hiljada` (exact noun + compound forms; avoids mangling `tisućljeće`)
    ("tisuća", "hiljada"),
    ("tisuće", "hiljade"),
    ("tisući", "hiljadi"),
    ("tisuću", "hiljadu"),
    ("tisućo", "hiljado"),
    ("tisućom", "hiljadom"),
    ("tisućama", "hiljadama"),
    ("tisućljetni", "hiljadugodišnji"),
    ("tisućljetna", "hiljadugodišnja"),
    ("tisućljetno", "hiljadugodišnje"),
    ("tisućljetne", "hiljadugodišnje"),
    ("tisućljetnu", "hiljadugodišnju"),
    ("tisućljetnog", "hiljadugodišnjeg"),
    ("tisućljetnoj", "hiljadugodišnjoj"),
    ("tisućljetnih", "hiljadugodišnjih"),
    ("tisućljetnim", "hiljadugodišnjim"),
    // `glazba` -> `muzika`, `glazbeni` -> `muzički`, `glazbenik` -> `muzičar`, `glazbenica` -> `muzičarka`
    ("glazba", "muzika"),
    ("glazbe", "muzike"),
    ("glazbi", "muzici"),
    ("glazbu", "muziku"),
    ("glazbo", "muziko"),
    ("glazbom", "muzikom"),
    ("glazbama", "muzikama"),
    ("glazbeni", "muzički"),
    ("glazbena", "muzička"),
    ("glazbeno", "muzičko"),
    ("glazbene", "muzičke"),
    ("glazbenu", "muzičku"),
    ("glazbenog", "muzičkog"),
    ("glazbenoga", "muzičkoga"),
    ("glazbenoj", "muzičkoj"),
    ("glazbenom", "muzičkom"),
    ("glazbenome", "muzičkome"),
    ("glazbenih", "muzičkih"),
    ("glazbenim", "muzičkim"),
    ("glazbenima", "muzičkima"),
    ("glazbenik", "muzičar"),
    ("glazbenika", "muzičara"),
    ("glazbeniku", "muzičaru"),
    ("glazbeniče", "muzičaru"),
    ("glazbenikom", "muzičarom"),
    ("glazbenici", "muzičari"),
    ("glazbenike", "muzičare"),
    ("glazbenicima", "muzičarima"),
    ("glazbenica", "muzičarka"),
    ("glazbenice", "muzičarke"),
    ("glazbenicu", "muzičarku"),
    ("glazbenicom", "muzičarkom"),
    ("glazbenicama", "muzičarkama"),
    // `tjedan` -> `nedelja` / `tjedni` -> `nedeljni` / `tjedno` -> `nedeljno`
    ("tjedan", "nedelja"),
    ("tjedna", "nedelje"),
    ("tjednu", "nedelji"),
    ("tjednom", "nedeljom"),
    ("tjedana", "nedelja"),
    ("tjedne", "nedelje"),
    ("tjednima", "nedeljama"),
    ("tjedni", "nedeljni"),
    ("tjedno", "nedeljno"),
    ("tjednog", "nedeljnog"),
    ("tjednoga", "nedeljnoga"),
    ("tjednoj", "nedeljnoj"),
    ("tjednih", "nedeljnih"),
    ("tjednim", "nedeljnim"),
    // `uvjet` -> `uslov` (exact noun/verb forms; participle `uvjetovan*` -> `uslovljen*` handled by stem)
    ("uvjet", "uslov"),
    ("uvjeta", "uslova"),
    ("uvjetu", "uslovu"),
    ("uvjetom", "uslovom"),
    ("uvjeti", "uslovi"),
    ("uvjete", "uslove"),
    ("uvjetima", "uslovima"),
    ("preduvjet", "preduslov"),
    ("preduvjeta", "preduslova"),
    ("preduvjetu", "preduslovu"),
    ("preduvjetom", "preduslovom"),
    ("preduvjeti", "preduslovi"),
    ("preduvjete", "preduslove"),
    ("preduvjetima", "preduslovima"),
    ("uvjetovati", "usloviti"),
    ("uvjetuje", "uslovljava"),
    ("uvjetuju", "uslovljavaju"),
    ("uvjetujem", "uslovljavam"),
    ("uvjetuješ", "uslovljavaš"),
    ("uvjetujemo", "uslovljavamo"),
    ("uvjetujete", "uslovljavate"),
    ("uvjetovao", "uslovio"),
    ("uvjetovala", "uslovila"),
    ("uvjetovali", "uslovili"),
    ("uvjetovalo", "uslovilo"),
    ("uvjetovale", "uslovile"),
    ("uvjetovanje", "uslovljavanje"),
    ("uvjetovanja", "uslovljavanja"),
    ("uvjetovanju", "uslovljavanju"),
    ("uvjetovanjem", "uslovljavanjem"),
    // Unambiguous Croatian month names -> standard Serbian month names.
    // Intentionally skipped to prevent Serbian false positives:
    // - `kolovoz` / `kolovoza` / `kolovozu` / `kolovozom`: standard Serbian for "roadway/carriageway" (e.g. "na kolovozu").
    // - `studeni` / `studenog` / `studenom`: standard Serbian adjective for "freezing cold" (e.g. "studeni vetar").
    // - `listopadn*` adjectives (`listopadne šume`, `listopadno drveće` = deciduous): protected by matching only exact noun forms of `listopad`.
    // - `travnjak` ("lawn"): protected by matching only exact noun forms of `travanj` and stem `travanjsk*`.
    // - `rujno` / `rujni` / `rujne` (poetic adjective "rosy/crimson", e.g. "rujno vino"): protected by matching only noun cases and `rujansk*`.
    ("siječanj", "januar"),
    ("siječnja", "januara"),
    ("siječnju", "januaru"),
    ("siječnjem", "januarom"),
    ("veljača", "februar"),
    ("veljače", "februara"),
    ("veljači", "februaru"),
    ("veljaču", "februar"),
    ("veljačom", "februarom"),
    ("ožujak", "mart"),
    ("ožujka", "marta"),
    ("ožujku", "martu"),
    ("ožujkom", "martom"),
    ("travanj", "april"),
    ("travnja", "aprila"),
    ("travnju", "aprilu"),
    ("travnjem", "aprilom"),
    ("svibanj", "maj"),
    ("svibnja", "maja"),
    ("svibnju", "maju"),
    ("svibnjem", "majem"),
    ("lipanj", "jun"),
    ("lipnja", "juna"),
    ("lipnju", "junu"),
    ("lipnjem", "junom"),
    ("srpanj", "jul"),
    ("srpnja", "jula"),
    ("srpnju", "julu"),
    ("srpnjem", "julom"),
    ("rujan", "septembar"),
    ("rujna", "septembra"),
    ("rujnu", "septembru"),
    ("rujnom", "septembrom"),
    ("listopad", "oktobar"),
    ("listopada", "oktobra"),
    ("listopadu", "oktobru"),
    ("listopadom", "oktobrom"),
    ("prosinac", "decembar"),
    ("prosinca", "decembra"),
    ("prosincu", "decembru"),
    ("prosincem", "decembrom"),
    // `vlak` -> `voz` (exact noun forms only; strictly protects Serbian `vlakno`/`vlakna`/`vlakana`/`vlaknima`/`vlaknasto` = fiber and `vlače`)
    ("vlak", "voz"),
    ("vlaka", "voza"),
    ("vlaku", "vozu"),
    ("vlakom", "vozom"),
    ("vlakovi", "vozovi"),
    ("vlakova", "vozova"),
    ("vlakove", "vozove"),
    ("vlakovima", "vozovima"),
    ("vlakić", "vozić"),
    ("vlakića", "vozića"),
    ("vlakiću", "voziću"),
    ("vlakićem", "vozićem"),
    ("vlakići", "vozići"),
    ("vlakiće", "voziće"),
    ("vlakićima", "vozićima"),
    // `kruh` -> `hleb` (exact noun forms only; strictly protects `kruška`, `krug`, `krut`, `kruna`, `krupan`, `kruše`)
    ("kruh", "hleb"),
    ("kruha", "hleba"),
    ("kruhu", "hlebu"),
    ("kruhom", "hlebom"),
    ("kruhovi", "hlebovi"),
    ("kruhova", "hlebova"),
    ("kruhove", "hlebove"),
    ("kruhovima", "hlebovima"),
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
    ("primijen", "primen"),
    ("primjen", "primen"),
    ("izmijen", "izmen"),
    ("izmjen", "izmen"),
    ("zamijen", "zamen"),
    ("zamjen", "zamen"),
    ("promijen", "promen"),
    ("promjen", "promen"),
    ("razmijen", "razmen"),
    ("razmjen", "razmen"),
    ("smijen", "smen"),
    ("smjen", "smen"),
    ("mijenj", "menj"),
    ("razumijev", "razumev"),
    ("razumjet", "razumet"),
    ("vidjel", "videl"),
    ("htjel", "htel"),
    ("željel", "želel"),
    ("živjel", "živel"),
    ("voljel", "volel"),
    ("smjel", "smel"),
    ("povjerenj", "poverenj"),
    ("vjerov", "verov"),
    ("vjeruj", "veruj"),
    ("pjesm", "pesm"),
    ("djevoj", "devoj"),
    ("dječ", "deč"),
    ("osjeć", "oseć"),
    ("osjet", "oset"),
    ("prisjeć", "priseć"),
    ("podsjeć", "podseć"),
    ("dosjeć", "doseć"),
    ("sjeć", "seć"),
    ("sjet", "set"),
    ("sjever", "sever"),
    ("prebjeg", "prebeg"),
    ("pobjeg", "pobeg"),
    ("pobjed", "pobed"),
    ("bjež", "bež"),
    ("svjedoč", "svedoč"),
    ("svjedok", "svedok"),
    ("vrijedn", "vredn"),
    ("vrjedn", "vredn"),
    ("grijanj", "grejanj"),
    ("zagrija", "zagreja"),
    ("ugrija", "ugreja"),
    ("ogrija", "ogreja"),
    ("vijek", "vek"),
    ("vjekov", "vekov"),
    ("vječn", "večn"),
    ("vječit", "večit"),
    ("tjelesn", "telesn"),
    ("odjeć", "odeć"),
    ("odjeven", "odeven"),
    ("odjevn", "odevn"),
    ("djelomič", "delimič"),
    ("djelotvor", "delotvor"),
    ("djelov", "delov"),
    ("djeluj", "deluj"),
    ("djelat", "delat"),
    ("djeljiv", "deljiv"),
    ("dijel", "del"),
    ("procvjet", "procvet"),
    ("rascvjet", "rascvet"),
    ("cvjet", "cvet"),
    ("cvijet", "cvet"),
    ("posjeć", "poseć"),
    ("posjet", "poset"),
    ("potjer", "poter"),
    ("otjer", "oter"),
    ("istjer", "ister"),
    ("natjer", "nater"),
    ("protjer", "proter"),
    ("utjer", "uter"),
    ("zatjer", "zater"),
    ("tjer", "ter"),
    ("procijen", "procen"),
    ("procjen", "procen"),
    ("ocijen", "ocen"),
    ("ocjen", "ocen"),
    ("precijen", "precen"),
    ("precjenj", "precenj"),
    ("potcijen", "potcen"),
    ("potcjenj", "potcenj"),
    ("ucijen", "ucen"),
    ("ucjen", "ucen"),
    ("cijen", "cen"),
    ("cijev", "cev"),
    ("cijel", "cel"),
    ("osvijetl", "osvetl"),
    ("osvjetl", "osvetl"),
    ("rasvijetl", "rasvetl"),
    ("rasvjetl", "rasvetl"),
    ("prosvijet", "prosvet"),
    ("prosvjet", "prosvet"),
    ("svijetl", "svetl"),
    ("svijet", "svet"),
    ("svjetl", "svetl"),
    ("svjetsk", "svetsk"),
    ("svijeć", "sveć"),
    ("svijest", "svest"),
    ("savjet", "savet"),
    ("izvješt", "izvešt"),
    ("izvijest", "izvest"),
    ("obavijest", "obavest"),
    ("obavješt", "obavešt"),
    ("navijest", "navest"),
    ("navješt", "navešt"),
    ("vijest", "vest"),
    ("upotrijeb", "upotreb"),
    ("upotrjeb", "upotreb"),
    ("pogriješ", "pogreš"),
    ("zgriješ", "zgreš"),
    ("sagriješ", "sagreš"),
    ("griješ", "greš"),
    ("razriješ", "razreš"),
    ("razrješ", "razreš"),
    ("riješ", "reš"),
    ("rješ", "reš"),
    ("rijek", "rek"),
    ("riječ", "reč"),
    ("rječnik", "rečnik"),
    ("spriječ", "spreč"),
    ("sprječ", "spreč"),
    ("prepriječ", "prepreč"),
    ("unaprijeđ", "unapređ"),
    ("unaprijed", "unapred"),
    ("prijedlog", "predlog"),
    ("prijevoz", "prevoz"),
    ("prijelaz", "prelaz"),
    ("prijenos", "prenos"),
    ("prijevrem", "prevrem"),
    ("prijevod", "prevod"),
    ("prijevar", "prevar"),
    ("prijekor", "prekor"),
    ("prijetnj", "pretnj"),
    ("prijetit", "pretit"),
    ("prijetio", "pretio"),
    ("prijet", "pret"),
    // Safe morphological stems for non-Serbian lexical leaks (longer prefixes first)
    ("tisućit", "hiljadit"),
    ("tisućark", "hiljadark"),
    ("glazbenikov", "muzičarev"),
    ("dvotjedn", "dvonedeljn"),
    ("trotjedn", "tronedeljn"),
    ("višetjedn", "višenedeljn"),
    ("tjednik", "nedeljnik"),
    ("tjednic", "nedeljnic"),
    ("neuvjetovan", "neuslovljen"),
    ("uvjetovanj", "uslovljavanj"),
    ("uvjetovan", "uslovljen"),
    ("bezuvjetn", "bezuslovn"),
    ("uvjetn", "uslovn"),
    ("siječanjsk", "januarsk"),
    ("ožujsk", "martovsk"),
    ("travanjsk", "aprilsk"),
    ("svibanjsk", "majsk"),
    ("lipanjsk", "junsk"),
    ("srpanjsk", "julsk"),
    ("rujansk", "septembarsk"),
    ("listopadsk", "oktobarsk"),
    ("prosinačk", "decembarsk"),
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

/// Normalizes common unambiguous Ijekavian and non-Serbian Croatian lexical forms
/// (including Ekavian stems, `tisuća` -> `hiljada`, `glazba` -> `muzika`,
/// `tjedan` -> `nedelja`, `uvjet` -> `uslov`, unambiguous month names, `vlak` -> `voz`,
/// `kruh` -> `hleb`) into standard Serbian Ekavian while preserving casing,
/// URLs, emails, HTML/ASS tags, markers, and Cyrillic/Latin script, and guarding
/// against Serbian false positives (`vlakno`, `kolovoz`, `studeni`, `listopadno`, `travnjak`).
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

fn conjunction_comma_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)(^|[.!?…]\s+|(?:^|\s)[—–-]\s+)(Tako da|Тако да|Ali|Али|Pa|Па|I|И|A|А)(,)(\s+)")
            .expect("valid conjunction comma regex")
    })
}

fn has_closing_parenthetical_comma(rest_of_sentence: &str) -> bool {
    let end = rest_of_sentence
        .find(['.', '!', '?', '…', '\n', ';'])
        .unwrap_or(rest_of_sentence.len());
    let clause_slice = &rest_of_sentence[..end];
    let Some(comma_idx) = clause_slice.find(',') else {
        return false;
    };
    let between = clause_slice[..comma_idx].trim();
    if between.is_empty() {
        return false;
    }
    let words: Vec<&str> = between.split_whitespace().collect();
    if words.len() > 3 {
        return false;
    }
    // If the slice between the conjunction and the next comma already introduces a finite subordinate/clause verb
    // rather than a short parenthetical adverbial ("naravno", "iskreno govoreći", "znate"), don't treat it as parenthetical.
    !words.iter().any(|w| {
        let norm = to_latin(w).to_lowercase();
        matches!(norm.as_str(), "da" | "što" | "jer" | "ako" | "kad" | "kada")
    })
}

fn clean_serbian_conjunction_commas_segment(text: &str) -> String {
    let re = conjunction_comma_regex();
    let mut out = String::with_capacity(text.len());
    let mut last_end = 0;

    for caps in re.captures_iter(text) {
        let whole = caps.get(0).unwrap();
        let prefix = caps.get(1).unwrap().as_str();
        let conj = caps.get(2).unwrap().as_str();
        let space = caps.get(4).unwrap().as_str();
        let after = &text[whole.end()..];

        if !has_closing_parenthetical_comma(after) {
            out.push_str(&text[last_end..whole.start()]);
            out.push_str(prefix);
            out.push_str(conj);
            out.push_str(space);
            last_end = whole.end();
        }
    }

    if last_end < text.len() {
        out.push_str(&text[last_end..]);
    }
    out
}

/// Removes unambiguous English-calque commas directly after sentence-initial Serbian conjunctions
/// (`I,`, `A,`, `Ali,`, `Pa,`, `Tako da,` and Cyrillic equivalents) while preserving genuine
/// parenthetical insertions (`I, naravno, idemo dalje.`, `Ali, iskreno govoreći, nije bilo lako.`).
pub fn clean_serbian_conjunction_commas(text: &str) -> String {
    let re = protected_span_regex();
    let mut out = String::with_capacity(text.len());
    let mut last_end = 0;

    for m in re.find_iter(text) {
        if m.start() > last_end {
            out.push_str(&clean_serbian_conjunction_commas_segment(
                &text[last_end..m.start()],
            ));
        }
        out.push_str(m.as_str());
        last_end = m.end();
    }

    if last_end < text.len() {
        out.push_str(&clean_serbian_conjunction_commas_segment(&text[last_end..]));
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
        assert_eq!(
            normalize_ekavian("Posjetite nas da biste vidjeli kako je cvjetala ova dolina."),
            "Posetite nas da biste videli kako je cvetala ova dolina."
        );
        assert_eq!(
            normalize_ekavian("Sunce će nas grijati cijelog vijeka."),
            "Sunce će nas grejati celog veka."
        );
        assert_eq!(
            normalize_ekavian("Šta vas je potjeralo da primijenite taj prijedlog i prijevod?"),
            "Šta vas je poteralo da primenite taj predlog i prevod?"
        );
        assert_eq!(
            normalize_ekavian("Njegovo tijelo, odjeća i sjećanje na djelo osvijetlili su izvještaj i savjet."),
            "Njegovo telo, odeća i sećanje na delo osvetlili su izveštaj i savet."
        );
        assert_eq!(
            normalize_ekavian("Moramo spriječiti greške, procijeniti štetu, obavijestiti sve, unaprijed upotrijebiti plan i riješiti problem."),
            "Moramo sprečiti greške, proceniti štetu, obavestiti sve, unapred upotrebiti plan i rešiti problem."
        );
        assert_eq!(
            normalize_ekavian("Sjedinjene Države i sjedinjenje ne treba mijenjati."),
            "Sjedinjene Države i sjedinjenje ne treba menjati."
        );
    }

    #[test]
    fn normalizes_expanded_serbian_lexical_families_and_cases() {
        // tisuća -> hiljada (singular, plural, cases, ordinals, casing, Cyrillic)
        assert_eq!(
            normalize_ekavian("Tisuća ljudi, dve tisuće dinara, tisuću puta, tisućama godina, po tisućiti put."),
            "Hiljada ljudi, dve hiljade dinara, hiljadu puta, hiljadama godina, po hiljaditi put."
        );
        assert_eq!(
            normalize_ekavian("TISUĆA i Tisućarka iz tisućljetnog grada."),
            "HILJADA i Hiljadarka iz hiljadugodišnjeg grada."
        );
        assert_eq!(
            normalize_ekavian("Тисућа људи, ТИСУЋЕ динара и тисућама година."),
            "Хиљада људи, ХИЉАДЕ динара и хиљадама година."
        );

        // glazba -> muzika, glazbeni -> muzički, glazbenik -> muzičar, glazbenica -> muzičarka
        assert_eq!(
            normalize_ekavian("Glazba je lepa; bez glazbe, u glazbi, volim glazbu i živim s glazbom."),
            "Muzika je lepa; bez muzike, u muzici, volim muziku i živim s muzikom."
        );
        assert_eq!(
            normalize_ekavian("Glazbeni festival, glazbena škola, glazbeno delo, glazbenih numera."),
            "Muzički festival, muzička škola, muzičko delo, muzičkih numera."
        );
        assert_eq!(
            normalize_ekavian("Glazbenik i glazbenica su sjajni glazbenici; pričamo sa glazbenikom."),
            "Muzičar i muzičarka su sjajni muzičari; pričamo sa muzičarom."
        );
        assert_eq!(
            normalize_ekavian("ГЛАЗБА, Глазби, глазбени и глазбеник."),
            "МУЗИКА, Музици, музички и музичар."
        );

        // tjedan -> nedelja / tjedni -> nedeljni / tjedno -> nedeljno
        assert_eq!(
            normalize_ekavian("Tjedan dana, tri tjedna, u tjednu, pet tjedana, tjednima čekamo."),
            "Nedelja dana, tri nedelje, u nedelji, pet nedelja, nedeljama čekamo."
        );
        assert_eq!(
            normalize_ekavian("Tjedni plan, tri puta tjedno, na tjednoj bazi, dvotjedni odmor i tjednik."),
            "Nedeljni plan, tri puta nedeljno, na nedeljnoj bazi, dvonedeljni odmor i nedeljnik."
        );
        assert_eq!(
            normalize_ekavian("ТЈЕДАН, Тједно и тједнима у тједнику."),
            "НЕДЕЉА, Недељно и недељама у недељнику."
        );

        // uvjet -> uslov, uvjetovan -> uslovljen, bezuvjetno -> bezuslovno, preduvjet -> preduslov
        assert_eq!(
            normalize_ekavian("Uvjet, bez uvjeta, po uvjetu, pod uvjetom, uvjeti, uvjete i uvjetima."),
            "Uslov, bez uslova, po uslovu, pod uslovom, uslovi, uslove i uslovima."
        );
        assert_eq!(
            normalize_ekavian("Uvjetovan refleks, uvjetovana odluka, uvjetovanost i neuvjetovano."),
            "Uslovljen refleks, uslovljena odluka, uslovljenost i neuslovljeno."
        );
        assert_eq!(
            normalize_ekavian("Bezuvjetna ljubav je preduvjet koji uvjetuje uspeh i uvjetovanje sa uvjetovanjima."),
            "Bezuslovna ljubav je preduslov koji uslovljava uspeh i uslovljavanje sa uslovljavanjima."
        );
        assert_eq!(
            normalize_ekavian("УВЈЕТ, Увјетован и безувјетно."),
            "УСЛОВ, Условљен и безусловно."
        );

        // vlak -> voz
        assert_eq!(
            normalize_ekavian("Vlak kasni; bez vlaka, u vlaku, idem vlakom, vlakovi i vlakovima."),
            "Voz kasni; bez voza, u vozu, idem vozom, vozovi i vozovima."
        );
        assert_eq!(
            normalize_ekavian("ВЛАК, Влакови и путујемо влаком."),
            "ВОЗ, Возови и путујемо возом."
        );

        // kruh -> hleb (plus completed hljeb forms)
        assert_eq!(
            normalize_ekavian("Kruh, parče kruha, o kruhu, sa kruhom, kruhovi i hljebom."),
            "Hleb, parče hleba, o hlebu, sa hlebom, hlebovi i hlebom."
        );
        assert_eq!(
            normalize_ekavian("КРУХ, Круха и са крухом."),
            "ХЛЕБ, Хлеба и са хлебом."
        );

        // Croatian pronouns & lexical forms
        assert_eq!(
            normalize_ekavian("Tko zna da li je netko ili nitko video svatko i taj krajolik?"),
            "Ko zna da li je neko ili niko video svako i taj pejzaž?"
        );
        assert_eq!(
            normalize_ekavian("Тко је видео повијесни крајолик?"),
            "Ко је видео историјски пејзаж?"
        );
    }

    #[test]
    fn normalizes_unambiguous_croatian_months_preserving_case_and_cyrillic() {
        assert_eq!(
            normalize_ekavian("Siječanj, u siječnju, 10. siječnja, siječanjski mraz."),
            "Januar, u januaru, 10. januara, januarski mraz."
        );
        assert_eq!(
            normalize_ekavian("Veljača, u veljači, 14. veljače, veljačom."),
            "Februar, u februaru, 14. februara, februarom."
        );
        assert_eq!(
            normalize_ekavian("Ožujak, u ožujku, 8. ožujka, ožujski dani."),
            "Mart, u martu, 8. marta, martovski dani."
        );
        assert_eq!(
            normalize_ekavian("Travanj, u travnju, 1. travnja, travanjska kiša."),
            "April, u aprilu, 1. aprila, aprilska kiša."
        );
        assert_eq!(
            normalize_ekavian("Svibanj, u svibnju, 1. svibnja, svibnjem, svibanjski praznik."),
            "Maj, u maju, 1. maja, majem, majski praznik."
        );
        assert_eq!(
            normalize_ekavian("Lipanj, u lipnju, 21. lipnja, lipanjsko sunce."),
            "Jun, u junu, 21. juna, junsko sunce."
        );
        assert_eq!(
            normalize_ekavian("Srpanj, u srpnju, 15. srpnja, srpanjska vrućina."),
            "Jul, u julu, 15. jula, julska vrućina."
        );
        assert_eq!(
            normalize_ekavian("Rujan, u rujnu, 1. rujna, rujanski rok."),
            "Septembar, u septembru, 1. septembra, septembarski rok."
        );
        assert_eq!(
            normalize_ekavian("Listopad, u listopadu, 5. listopada, listopadski događaj."),
            "Oktobar, u oktobru, 5. oktobra, oktobarski događaj."
        );
        assert_eq!(
            normalize_ekavian("Prosinac, u prosincu, 31. prosinca, prosinački sneg."),
            "Decembar, u decembru, 31. decembra, decembarski sneg."
        );
        // ALL CAPS & Cyrillic months
        assert_eq!(
            normalize_ekavian("SIJEČANJ, VELJAČA, PROSINAC"),
            "JANUAR, FEBRUAR, DECEMBAR"
        );
        assert_eq!(
            normalize_ekavian("Сијечањ, ВЕЉАЧА, у ожујку, 15. рујна и у просинцу."),
            "Јануар, ФЕБРУАР, у марту, 15. септембра и у децембру."
        );
    }

    #[test]
    fn guards_against_serbian_false_positives_and_preserves_protected_spans() {
        // `vlakno` / `vlakana` / `vlaknima` = fiber (MUST NOT become `vozno` / `vozana`!)
        let fiber_latin = "Ova hrana ima puno vlakana, jedno vlakno, bogata vlaknima i vlaknasta je, dok konji vlače.";
        assert_eq!(normalize_ekavian(fiber_latin), fiber_latin);
        let fiber_cyr = "Влакно, влакана и влакнима богато јело.";
        assert_eq!(normalize_ekavian(fiber_cyr), fiber_cyr);

        // `kolovoz` in Serbian = roadway/carriageway (MUST NOT become `avgust`!)
        let road_latin = "Na kolovozu ima leda, kolovoz je klizav, vozilo je sletelo sa kolovoza.";
        assert_eq!(normalize_ekavian(road_latin), road_latin);
        let road_cyr = "На коловозу има поледице и коловоз је мокар.";
        assert_eq!(normalize_ekavian(road_cyr), road_cyr);

        // `studeni` in Serbian = freezing cold adjective (MUST NOT become `novembar`!)
        let cold_latin = "Studeni vetar duva po studenom vremenu od studenog jutra.";
        assert_eq!(normalize_ekavian(cold_latin), cold_latin);
        let cold_cyr = "Студени ветар дува целог дана.";
        assert_eq!(normalize_ekavian(cold_cyr), cold_cyr);

        // `listopadne šume` / `listopadno drveće` (deciduous), `travnjak` (lawn), `rujno vino` (rosy)
        let botany = "Listopadne šume, listopadno drveće, zeleni travnjak i rujno vino.";
        assert_eq!(normalize_ekavian(botany), botany);

        // `kruška`, `krug`, `krut`, `kruna`, `krupan`, `kruše` (MUST NOT be affected by `kruh`)
        let kruh_neighbors = "Kruška, krug, krut stav, kruna, krupan plod i zrna se kruše.";
        assert_eq!(normalize_ekavian(kruh_neighbors), kruh_neighbors);

        // `tisućljeće` MUST NOT be corrupted into invalid `hiljadljeće`
        assert_eq!(normalize_ekavian("Novo tisućljeće"), "Novo tisućljeće");

        // `Sjedinjene` / `sjedinjenje`
        assert_eq!(
            normalize_ekavian("Sjedinjene Države i sjedinjenje"),
            "Sjedinjene Države i sjedinjenje"
        );

        // Protected spans (URLs, emails, HTML/ASS tags, bracket markers) untouched alongside converted words
        let mixed = "<i>Glazba za tisuću ljudi na https://example.com/vlak/tisuća/glazba ili glazba@vlak.hr [GLAZBA] {\\an8}</i>";
        assert_eq!(
            normalize_ekavian(mixed),
            "<i>Muzika za hiljadu ljudi na https://example.com/vlak/tisuća/glazba ili glazba@vlak.hr [GLAZBA] {\\an8}</i>"
        );
    }

    #[test]
    fn lexical_tables_have_unique_lowercase_keys() {
        use std::collections::HashSet;

        let mut exact_keys = HashSet::new();
        for &(src, dst) in EXACT_IJEKAVIAN_TO_EKAVIAN {
            assert!(!src.is_empty() && !dst.is_empty());
            assert_eq!(src, src.to_lowercase(), "exact key must be lowercase: {src}");
            assert_ne!(src, dst, "no-op exact mapping: {src}");
            assert!(
                exact_keys.insert(src),
                "duplicate key in EXACT_IJEKAVIAN_TO_EKAVIAN: {src}"
            );
        }

        let mut stem_keys = HashSet::new();
        for (idx, &(src, dst)) in STEM_IJEKAVIAN_TO_EKAVIAN.iter().enumerate() {
            assert!(!src.is_empty() && !dst.is_empty());
            assert_eq!(src, src.to_lowercase(), "stem key must be lowercase: {src}");
            assert_ne!(src, dst, "no-op stem mapping: {src}");
            assert!(
                stem_keys.insert(src),
                "duplicate key in STEM_IJEKAVIAN_TO_EKAVIAN: {src}"
            );
            for &(earlier, _) in &STEM_IJEKAVIAN_TO_EKAVIAN[..idx] {
                assert!(
                    !src.starts_with(earlier),
                    "earlier stem `{earlier}` shadows longer stem `{src}` in STEM_IJEKAVIAN_TO_EKAVIAN"
                );
            }
        }
    }

    #[test]
    fn converts_script_values_in_place_across_variant_maps() {
        use std::collections::BTreeMap;

        let mut variants = BTreeMap::from([
            ("natural", "Đorđe gleda film u Nišu na https://example.com/ljubav.".to_string()),
            ("formal", "Gospodin Đorđe prisustvuje projekciji u Nišu.".to_string()),
            ("empty", String::new()),
        ]);

        convert_script_values(variants.values_mut(), SerbianScript::Cyrillic);
        assert_eq!(
            variants.get("natural").map(String::as_str),
            Some("Ђорђе гледа филм у Нишу на https://example.com/ljubav.")
        );
        assert_eq!(
            variants.get("formal").map(String::as_str),
            Some("Господин Ђорђе присуствује пројекцији у Нишу.")
        );
        assert_eq!(variants.get("empty").map(String::as_str), Some(""));

        convert_script_values(variants.values_mut(), SerbianScript::Latin);
        assert_eq!(
            variants.get("natural").map(String::as_str),
            Some("Đorđe gleda film u Nišu na https://example.com/ljubav.")
        );
        assert_eq!(
            variants.get("formal").map(String::as_str),
            Some("Gospodin Đorđe prisustvuje projekciji u Nišu.")
        );
        assert_eq!(variants.get("empty").map(String::as_str), Some(""));
    }

    #[test]
    fn cleans_stray_english_conjunction_commas_while_preserving_parentheticals() {
        // Unambiguous English calques -> stripped (Latin & Cyrillic)
        assert_eq!(
            clean_serbian_conjunction_commas("I, mislim da možemo."),
            "I mislim da možemo."
        );
        assert_eq!(
            clean_serbian_conjunction_commas("Tako da, težak je poraz."),
            "Tako da težak je poraz."
        );
        assert_eq!(
            clean_serbian_conjunction_commas("Ali, nismo odustali."),
            "Ali nismo odustali."
        );
        assert_eq!(
            clean_serbian_conjunction_commas("Pa, dobro je počelo. A, onda smo stali."),
            "Pa dobro je počelo. A onda smo stali."
        );
        assert_eq!(
            clean_serbian_conjunction_commas("И, мислим да можемо. Тако да, идемо даље."),
            "И мислим да можемо. Тако да идемо даље."
        );
        assert_eq!(
            clean_serbian_conjunction_commas("— I, tako smo rešili problem."),
            "— I tako smo rešili problem."
        );

        // Genuine Serbian parenthetical insertions -> preserved intact
        assert_eq!(
            clean_serbian_conjunction_commas("I, naravno, idemo dalje."),
            "I, naravno, idemo dalje."
        );
        assert_eq!(
            clean_serbian_conjunction_commas("Ali, iskreno govoreći, nije bilo lako."),
            "Ali, iskreno govoreći, nije bilo lako."
        );
        assert_eq!(
            clean_serbian_conjunction_commas("Pa, znate, sezona je duga."),
            "Pa, znate, sezona je duga."
        );
        assert_eq!(
            clean_serbian_conjunction_commas("И, наравно, успели смо."),
            "И, наравно, успели смо."
        );
    }
}
