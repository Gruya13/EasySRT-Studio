# EasySRT Studio

**EasySRT Studio** je profesionalni *offline-first* desktop studio za automatsku transkripciju audio i video fajlova, pametnu segmentaciju titlova, prevođenje i jezičko poliranje na srpski jezik (ekavski standard, **latinica** i **ćirilica**), uvoz postojećih `.srt`/`.vtt` titlova i višestruki izvoz (`.srt`, `.vtt`, `.txt`).

Izgrađen pomoću **Tauri v2**, **Rust**, **React 19**, **TypeScript**, **Vite** i **Tailwind CSS v4** tehnologija — bez potrebe za Python-om, Docker-om ili ručnom instalacijom NVIDIA CUDA Toolkit-a.

Detaljan tehnički opis sistema nalazi se u dokumentu [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

---

## Ključne funkcionalnosti

- **100% Offline AI Pipeline (NVIDIA CUDA / Vulkan / CPU)**:
  - **FFmpeg** automatska normalizacija bilo kog audio/video fajla (`.mp3`, `.wav`, `.m4a`, `.flac`, `.ogg`, `.aac`, `.mp4`, `.mkv`, `.webm`) u `16 kHz mono PCM WAV`.
  - **whisper.cpp (`whisper-cli`) + Silero VAD v6.2.0**: Precizna transkripcija na nivou pojedinačnih reči (`-ojf`) uz eliminaciju halucinacija u tišini i podršku za korisnički rečnik pojmova (`--prompt`).
  - **Sekvencijalno upravljanje VRAM memorijom**: `whisper-cli` se u potpunosti gasi i oslobađa 100% GPU VRAM-a pre nego što se pokrene `llama-server`, čime se omogućava rad vrhunskih modela čak i na laptop grafičkim karticama sa **4 GB VRAM-a** (npr. RTX 3050).
- **Pametna segmentacija i profili titlova**:
  - **Standard (TV / Film)**: do 2 linije, maks. 42 karaktera po liniji, trajanje 1.0s – 7.0s, maks. 17.0 CPS (*Characters Per Second*).
  - **Reels / TikTok / Shorts**: 1 kratka dinamična linija, maks. 20 karaktera, trajanje 0.6s – 3.5s, maks. 18.0 CPS.
  - **Custom**: Potpuna kontrola nad brojem linija, dužinom reda, minimalnim/maksimalnim trajanjem i CPS pragom, uz trenutno preračunavanje bez ponovne transkripcije.
- **ID-zaključano kontekstualno prevođenje i poliranje srpskog jezika**:
  - Prevođenje u paketima (*batch*) sa klizećim kontekstualnim prozorom (**3 prethodna + N ciljnih + 3 naredna titla**) i automatskom rekurzivnom podelom paketa (*sub-batch retry*) koja garantuje 1:1 očuvanje ID-jeva i vremenskih oznaka.
  - **Lokalni LLM (`llama-server`)** ili **Cloud OpenAI-kompatibilni API (DeepSeek / Custom)** sa eksplicitnom zaštitom privatnosti (`OnlineConfirmModal`).
  - **Poliranje srpskog ASR izlaza**: Automatska ispravka grešaka prepoznavanja govora, interpunkcije i normalizacija u prirodni srpski ekavski standard.
- **Deterministička konverzija pisma (Latinica ↔ Ćirilica)**:
  - Trenutna konverzija jednim klikom uz pravilno kapitalizovanje digrafa (`Љубав` ↔ `Ljubav`, `ЉУБАВ` ↔ `LJUBAV`, `Његош` ↔ `Njegoš`, `Џеп` ↔ `Džep`).
  - Rečnik morfoloških izuzetaka na granicama prefiksa (`injekcija` → `инјекција`, `konjugacija` → `конјугација`, `nadživeti` → `надживети`, `podžanr` → `поджанр`, `Tanjug` → `Танјуг`) i očuvanje HTML/ASS tagova (`<i>`, `{\an8}`), URL adresa i `[...]` oznaka.
- **Direktan uvoz `.SRT` i `.VTT` titlova**:
  - Prevucite postojeći `.srt` ili `.vtt` fajl (ili kliknite na **„Uvezi .SRT / .VTT"**) za direktno prevođenje, lekturu, preslovljavanje u ćirilicu/latinicu, pretragu i zamenu ili izvoz.
- **Interaktivni Studio Editor**:
  - Tabelarni prikaz sa inline izmenom izvornog i prevedenog teksta, CPS bedževima i upozorenjima (`high_cps`, `line_too_long`, `duration_too_short`, `duration_too_long`).
  - **Audio plejer po segmentu**: Preslušavanje tačnog isečka `[startMs, endMs]` sa automatskim zaustavljanjem i finim pomeranjem vremena (`-0.1s` / `+0.1s`).
  - **Pretraga i zamena (`Ctrl+H`)**, **Undo / Redo (`Ctrl+Z` / `Ctrl+Y`)**, deljenje i spajanje titlova.
  - **Atomični checkpoint-i**: Svaka faza i svaki prevedeni paket se automatski čuvaju u `data/checkpoints/<job_id>.easysrt.json` za siguran nastavak prekinutih poslova.

---

## Hardverski VRAM nivoi i katalog modela

EasySRT Studio pri pokretanju automatski detektuje NVIDIA GPU putem `nvidia-smi` i preporučuje optimalni paket binarnih alata i modela iz verifikovanog kataloga (`src-tauri/resources/catalog.json`):

| Hardverski nivo | Detektovani VRAM | Preporučeni Whisper model | Preporučeni lokalni LLM za prevod i lekturu |
| :--- | :--- | :--- | :--- |
| **12+ GB VRAM** (`vram12GbPlus`) | `>= 11.000 MiB` (npr. RTX 3060 12GB, 4070 Ti, 3090, 4090) | **Whisper Large V3 Turbo (FP16)** (`1.62 GB`) | **Gemma 4 12B IT (Official QAT Q4_0)** (`6.98 GB`) ili **Gemma 3 12B IT (Q4_K_M)** (`7.30 GB`) |
| **8 GB VRAM** (`vram8Gb`) | `5.500 – 10.999 MiB` (npr. RTX 3060 Ti, 3070, 4060 8GB) | **Whisper Large V3 Turbo (Q5_0)** (`574 MB`) | **Gemma 4 E4B IT (Official QAT Q4_0)** (`5.15 GB`) ili **Gemma 3 4B IT (Q8_0)** (`4.13 GB`) |
| **4 GB VRAM** (`vram4Gb`) | `3.500 – 5.499 MiB` (npr. RTX 3050 4GB Laptop, GTX 1650) | **Whisper Large V3 Turbo (Q5_0)** (`574 MB`) | **Gemma 3 4B IT (Q4_K_M)** (`2.49 GB`) ili **Qwen 3 4B Instruct 2507 (Q4_K_M)** (`2.50 GB`) |
| **Samo CPU** (`cpuOnly`) | `< 3.500 MiB` ili bez NVIDIA GPU-a | **Whisper Medium (Q5_0)** (`539 MB`) | **Gemma 3 4B IT (Q4_K_M)** (CPU režim) ili **DeepSeek Cloud API** |

> **Napomena za Windows CUDA 12.4:** Katalog automatski preuzima i izoluje `whisper-cli.exe` (`bin/whisper/`) i `llama-server.exe` sa pratećim `cudart64_12.dll` / `cublas64_12.dll` bibliotekama (`bin/llama/`). Korisniku je potreban samo standardni NVIDIA drajver.

---

## Čarobnjak za prvo pokretanje (First-Run Setup Wizard)

Pri prvom pokretanju aplikacije otvara se **Čarobnjak za podešavanje**:
1. **Detekcija hardvera i režima skladišta**: Prikazuje naziv grafičke kartice, raspoloživi VRAM, preporučeni profil i aktivni režim (`Instalirani` ili `Portabilni`).
2. **Izbor lokacije za modele**: Možete zadržati podrazumevani `models/` direktorijum ili izabrati bilo koji prilagođeni folder na drugom disku (`modelsDirOverride`).
3. **Instalacija preporučenog paketa jednim klikom**: Preuzima `ffmpeg`, `whisper.cpp`, `llama.cpp`, `Silero VAD v6.2.0`, preporučeni Whisper model i lokalni GGUF LLM uz podršku za nastavak prekinutog preuzimanja (*HTTP Range resume*) i strogu **SHA-256** verifikaciju integriteta.
4. **Uvoz sopstvenih modela**: U podešavanjima u svakom trenutku možete uvesti i sopstveni `.bin` (Whisper) ili `.gguf` (LLM) model sa diska.

---

## Portabilni (Portable) vs. Instalirani (Installer) režim

EasySRT Studio podržava dva potpuno nezavisna režima rada:

1. **Instalirani režim (`NSIS Installer` na Windows-u / `AppImage` na Linux-u)**:
   - Podaci, binarni alati i modeli čuvaju se u standardnom korisničkom direktorijumu operativnog sistema (`%LOCALAPPDATA%\com.easysrt.studio` na Windows-u ili `~/.local/share/com.easysrt.studio` na Linux-u).
   - API ključevi se čuvaju u sistemskom sefu (**Windows Credential Manager** / **Linux Keyutils**).
2. **Portabilni režim (`EasySRT-Studio-windows-x64-portable.zip`)**:
   - Aktivira se automatski kada se pored izvršnog fajla (`EasySRT Studio.exe`) nalazi fajl **`portable.flag`**.
   - Svi direktorijumi (`bin/`, `models/`, `data/checkpoints/`, `config.json` i maskirani `.credentials.json`) čuvaju se lokalno u istom folderu, pa ceo studio možete pokretati direktno sa eksternog SSD-a ili USB diska bez ostavljanja tragova u sistemu.

---

## Podešavanje DeepSeek API-ja i Cloud prevodioca

Pored 100% lokalnog `llama-server` prevodioca, EasySRT Studio podržava **DeepSeek API** (`deepseek-chat`) i bilo koji **Custom OpenAI-kompatibilni endpoint** (npr. OpenRouter, Together, lokalni Ollama/vLLM):

1. Otvorite **Podešavanja (Settings) → Cloud API i modeli**.
2. Izaberite provajdera **DeepSeek API** (ili **Custom OpenAI API**).
3. Unesite svoj API ključ i kliknite na **Sačuvaj ključ** (ključ se bezbedno skladišti u OS Keyring-u ili portabilnom `.credentials.json` sa `0600` dozvolama — nikada u `config.json`).
4. Kada je izabran online provajder, gornja traka prikazuje bedž **`ONLINE • DeepSeek API`**, a pre prvog slanja teksta u oblak aplikacija traži vašu eksplicitnu potvrdu privatnosti (uz opciju *„Zapamti za ovu sesiju"*).

---

## Prečice na tastaturi (Keyboard Shortcuts)

U glavnoj tabeli titlova (**Studio Editor**) dostupne su sledeće prečice:

| Prečica | Akcija |
| :--- | :--- |
| **`Ctrl + F`** | Fokusira polje za brzu pretragu titlova |
| **`Ctrl + H`** | Otvara i fokusira traku za **Pretragu i zamenu (Search & Replace)** (`Izvor`, `Prevod` ili `Oba`, uz `Aa` opciju razlikovanja velikih/malih slova) |
| **`Ctrl + Z`** | **Opozovi (Undo)** poslednju izmenu titlova (do 50 koraka istorije) |
| **`Ctrl + Y`** ili **`Ctrl + Shift + Z`** | **Ponovi (Redo)** opozvanu izmenu titlova |
| **`Space`** | Reprodukuje / pauzira audio isečak trenutno izabranog titla (`[startMs, endMs]`) kada fokus nije u polju za unos teksta |
| **`Alt + ArrowDown`** | Prelazi na sledeći vidljivi titl u tabeli i reprodukuje njegov audio segment |
| **`Alt + ArrowUp`** | Prelazi na prethodni vidljivi titl u tabeli i reprodukuje njegov audio segment |

---

## Razvoj i testiranje (Development Commands)

### Preduslovi
- **Node.js** 22+ i `npm`
- **Rust** (`stable` toolchain) i `cargo`
- Na Linux-u (Ubuntu/Debian): `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`, `ffmpeg`

### Komande za razvoj i proveru

```bash
# 1. Instalacija frontend zavisnosti
npm install

# 2. TypeScript provera tipova
npm run typecheck

# 3. Pokretanje automatizovanih frontend unit testova (Node test runner)
npm test

# 4. Produkcijski build frontend-a
npm run build

# 5. Pokretanje Tauri desktop aplikacije u razvojnom režimu
npm run tauri dev
```

### Provera i testiranje Rust Tauri backend-a

```bash
cd src-tauri

# Brza provera kompilacije svih targeta
cargo check --all-targets

# Pokretanje kompletne unit i End-to-End test suite-e
cargo test

# Statika analiza koda (Clippy)
cargo clippy --all-targets

# Kompajliranje produkcijskog binarnog fajla
cargo build --release
```

---

## Potpisivanje izdanja i Auto-Updater (GitHub Releases)

[`src-tauri/tauri.conf.json`](src-tauri/tauri.conf.json) je konfigurisan za `tauri-plugin-updater` i proverava `https://github.com/Gruya13/EasySRT-Studio/releases/latest/download/latest.json`.

Pre objavljivanja potpisanog izdanja:

1. **Generišite Tauri Minisign par ključeva**:
   ```bash
   npx tauri signer generate -w ~/.tauri/easysrt-studio.key
   ```
2. **Ažurirajte javni ključ**: Zamenite vrednost `plugins.updater.pubkey` u [`src-tauri/tauri.conf.json`](src-tauri/tauri.conf.json) sadržajem generisanog `~/.tauri/easysrt-studio.key.pub` fajla.
3. **Dodajte GitHub Actions Secrets** (*Settings → Secrets and variables → Actions*):
   - `TAURI_SIGNING_PRIVATE_KEY`: sadržaj privatnog ključa `~/.tauri/easysrt-studio.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: lozinka postavljena pri generisanju ključa
4. **Objavite novu verziju**:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
   GitHub Actions workflow (`.github/workflows/release.yml`) će automatski izvršiti `npm test` i `cargo test`, izgraditi i potpisati Windows NSIS instaler, Windows Portable ZIP (`EasySRT-Studio-windows-x64-portable.zip`) i Linux AppImage, i direktno objaviti `latest.json` (`releaseDraft: false`).
