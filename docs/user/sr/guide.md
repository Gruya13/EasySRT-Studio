🌐 [English](../en/guide.md) | **Srpski** · [← Nazad na README](../../../README.sr.md) · [Rešavanje problema i FAQ](troubleshooting.md)

# EasySRT Studio — Korisnički vodič

Sve što vam je potrebno za instalaciju programa EasySRT Studio, izbor idealnog AI režima za vaš računar, obradu titlova na interaktivnom tajmlajnu i izvoz čistih fajlova spremnih za montažu i emitovanje.

---

## 1. Instalacija i prvo pokretanje

### Windows 10 / 11 (x64)
1. Preuzmite **`EasySRT.Studio_<version>_x64-setup.exe`** sa stranice [GitHub Releases](https://github.com/Gruya13/EasySRT-Studio/releases/latest) i pokrenite instalaciju.
2. Tokom instalacije program automatski postavlja FFmpeg, odgovarajuće Whisper komponente za vašu grafičku karticu (CUDA 12.4 GPU ili CPU), Silero VAD i podrazumevani **Whisper Large V3 Turbo (Q5_0)** model za prepoznavanje govora.
3. Pri prvom pokretanju **Čarobnjak za podešavanje (Setup Wizard)** prepoznaje memoriju vaše grafičke kartice (VRAM) i nudi preuzimanje besplatnog lokalnog modela za prevođenje ili povezivanje Cloud API ključa.

### Linux (x64)
1. Preuzmite **`EasySRT.Studio_<version>_amd64.AppImage`**, dodelite dozvolu za izvršavanje (`chmod +x EasySRT.Studio_*.AppImage`) i pokrenite aplikaciju.
2. Uverite se da je `ffmpeg` instaliran na sistemu (ili preuzmite komponente direktno kroz **Settings → Setup Wizard**).
3. Za GPU ubrzanje na Linux-u (NVIDIA, AMD, Intel) možete izabrati Vulkan komponente u Čarobnjaku za podešavanje ili raspakovati opcioni paket `whisper-bin-ubuntu-vulkan-x64.tar.gz` sa stranice izdanja.

---

## 2. Režimi rada i preporuke za hardver

Režim rada možete promeniti u bilo kom trenutku u panelu **Podešavanja / Settings** (`Obrada` / `Cloud API`):

| Režim rada | Najbolji izbor za | Preporučena konfiguracija |
| :--- | :--- | :--- |
| **100% Oflajn (Lokalni AI)** | Poverljive snimke klijenata, intervjue pod NDA ugovorom i rad bez troškova API-ja | **4 GB VRAM:** `Gemma 3 4B (Q4_K_M)` ili `Qwen 3 4B`<br>**8 GB VRAM:** `Gemma 4 E4B (QAT Q4_0)` ili `Gemma 3 4B (Q8_0)`<br>**12 GB+ VRAM:** `Gemma 4 12B` ili `Gemma 3 12B` |
| **Hybrid Turbo (GPU + Cloud API)** | Brzu obradu više fajlova (do **28x brže od snimka**) i najprirodniji stil prevoda | Lokalni GPU Whisper (`Large V3 Turbo Q5_0`) + **DeepSeek API** (`deepseek-chat`) ili bilo koji OpenAI-kompatibilni HTTPS servis |
| **Samo transkripcija (Bez LLM-a)** | Titlove na izvornom jeziku kada prevođenje ili lektura nisu potrebni | `Whisper Large V3 Turbo (Q5_0)` na GPU-u ili `Whisper Medium (Q5_0)` na računarima bez zasebne grafičke kartice |

- **Sekvencijalna zaštita VRAM memorije:** U oflajn režimu Whisper i lokalni LLM se nikada ne učitavaju u VRAM istovremeno — Whisper završava rad i oslobađa 100% memorije grafičke kartice pre nego što se pokrene `llama-server`.
- **Pametna paketna (Batch) obrada:** Kod više fajlova u oflajn režimu prvo se transkribuju svi snimci, a zatim se jedan zagrejan lokalni LLM koristi za ceo niz; u Cloud režimu GPU transkripcija narednog fajla teče paralelno dok se prethodni fajl prevodi.

---

## 3. Uvoz medija, pametna analiza i pravila segmentacije

- **Podržani audio i video formati:** `.mp3`, `.wav`, `.m4a`, `.flac`, `.ogg`, `.aac`, `.opus`, `.mp4`, `.mkv`, `.webm`.
- **Direktan uvoz titlova:** Prevucite postojeće `.srt` ili `.vtt` fajlove radi prevođenja, srpske lekture, preslovljavanja, skraćivanja prebrzih titlova ili doterivanja tajminga na tajmlajnu.
- **Pozadinska analiza snimka (Intake Probe):** Čim ubacite audio ili video u listu, EasySRT Studio tiho u pozadini procenjuje broj govornika, jezik i temu snimka (i automatski se zaustavlja u deliću sekunde čim kliknete na pokretanje obrade kako nikada ne bi usporio glavni posao).
- **Opcije formatiranja i prelamanja:**
  - **Profili:** `Standard` (`2 × 42` znaka za TV/film/YouTube), `Reels / Shorts` (`1 × 20` za kratke vertikalne forme) ili `Custom` (`1–4` balansirana reda uz sopstvene granice znakova i CPS brzine).
  - **Novi titl pri promeni govornika:** Akustički detektor automatski započinje nov titl kada progovori drugi sagovornik.
  - **Uklanjanje poštapalica i sečenje na zapeti:** Opciono čisti uzrečice i oklevanja (*ovaj, znači, uh, um…*), drži uvodne veznike vezane za nastavak misli i prirodno deli duge rečenice posle zapete.
  - **Ispravka izvornog transkripta (`correctSourceAsr`):** Tokom prevođenja ispravlja očigledne fonetske greške prepoznavanja govora u koloni originala bez menjanja smisla (**automatski je uključeno kada je aktivan Cloud API provajder poput DeepSeek ili Custom OpenAI**).
  - **Rečnik termina i automatski mini-pojmovnik:** Unesite stručne nazive, imena brendova ili parove `Izvor => Prevod` za Whisper (uz detektovani kontekst teme) i prevodioca, uz automatsku pre-prevodnu ekstrakciju domenskih idioma pre prvog paketa.

---

## 4. Cinema dvostruki editor, tajmlajn i prečice na tastaturi

Radni prostor spaja paralelni editor za **Original** i **Prevod** (uz nezavisno podešavanje veličine fonta i automatsko čuvanje izmena na svakih `600 ms`) sa trokanalnim striming **Waveform tajmlajnom**:

- **Prati plejhed (`Follow Playhead`) i zadržavanje titla preko pauza:** Podrazumevano aktivno dugme na traci tajmlajna automatski skroluje pregled tokom uvećanja (`100%–2000%`) i zadržava aktivni titl prikazan i preko kratkih pauza bez treperenja na „Nema aktivnog titla“.
- **Ekskluzivni `Space` transport:** Pritiskom na `Space` / `Shift+Space` van tekstualnih polja uvek pokrećete/zaustavljate tajmlajn (čak i ako je fokus bio na dugmetu), dok modalni dijalozi normalno zadržavaju fokus tastature.

| Prečica / Akcija | Dejstvo |
| :--- | :--- |
| `Space` | Pusti / Pauziraj zvuk na trenutnoj poziciji markera |
| `Shift + Space` | Pusti snimak od samog početka (`00:00.000`) |
| `←` / `→` | Pomeri marker unazad / unapred za 1 frejm (`40 ms`) |
| `Shift + ←` / `Shift + →` | Skoči unazad / unapred za `1 sekundu` |
| `↑` / `↓` | Pređi na sledeći / prethodni titl i odmah ga preslušaj |
| `C` | Preseci titl tačno na poziciji markera (uz čist razmak disanja od `80 ms`) |
| `Alt + 1` / `Alt + 2` | Fokusiraj polje **Original** (`Alt+1`) ili **Prevod** (`Alt+2`) aktivnog titla |
| `Ctrl + S` / `Ctrl + Enter` | Sačuvaj izmenu u polju za tekst (`Ctrl + Enter` ujedno vraća fokus na tajmlajn) |
| `Esc` | Izađi iz polja za tekst nazad na prečice tajmlajna |
| `Ctrl + Z` / `Ctrl + Y` | Opozovi (Undo) / Ponovi (Redo) izmene teksta i tajmlajna (`Ctrl + Shift + Z` takođe ponavlja) |
| **Točkić miša na tajmlajnu** | Glatko zumiranje oko kursora od **100% do 2000%** |
| **Prevlačenje leve / desne ivice titla** | Precizno podešavanje početka i kraja titla direktno prema zvučnom talasu |

---

## 5. Srpski jezički alati i izvoz titlova

- **Odvojeni stil i obraćanje (`Auto` / `Persiranje (Vi)` / `Neformalno (ti)`):** Nezavisno birajte govorni ili formalni ton od persiranja (`Vi`) ili obraćanja na `ti` (podrazumevani `Auto` bira `Vi` za intervjue i konferencije, a `ti` za podkaste), uz pravopisna pravila protiv engleskih kalkova i suvišnih zapeta posle početnih veznika (`I,`, `Tako da,`).
- **Trenutna promena stila `Prirodni` ↔ `Formalni` (`0 ms`):** Uporedite ili jednim klikom prebacite između živog govornog srpskog jezika (`Natural`) i zvaničnog/televizijskog registra (`Formal`). Keširane varijante se menjaju trenutno uz čuvanje vaših ručnih izmena za svaki stil posebno.
- **Ekavski standard i determinističko preslovljavanje `Latinica ↔ Ćirilica`:** Ugrađeni normalizator osigurava čist ekavski izgovor i preslovljava tekst uz pravilno pisanje digrafa (`Ljubav` ↔ `Љубав`, `LJUBAV` ↔ `ЉУБАВ`), morfološke izuzetke na granici prefiksa (`injekcija` → `инјекција`, `konjugacija` → `конјугација`, `Tanjug` → `Танјуг`) i netaknute HTML/ASS oznake i linkove.
- **Auto-Fit CPS jednim klikom:** Ciljano sažima samo one titlove koji prelaze dozvoljenu brzinu čitanja (`high_cps` / `line_too_long`), dok ostale titlove ne dira.
- **Višeformati izvoz:** Izvezite **`.SRT`**, **`.VTT`** ili čist **`.TXT`** transkript, uz opcioni **UTF-8 BOM** (za starije Windows programe za montažu) i opcioni dvostruki izvoz (prevedeni `.srt` + izvorni `.srt` odjednom).
