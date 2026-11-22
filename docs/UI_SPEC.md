# EasySRT Studio — UI/UX Specifikacija i Vodič za Implementaciju

Ovaj dokument predstavlja kompletnu tehničko-dizajnersku specifikaciju novog korisničkog interfejsa (**UI/UX**) za **EasySRT Studio**, nastalu na osnovu interaktivno dizajniranog i vizuelno verifikovanog prototipa [`mockup/index.html`](../mockup/index.html). Namenjen je za direktnu implementaciju u React 19 + TypeScript + Tailwind CSS v4 + Zustand (`src/stores/appStore.ts`) + Tauri v2 okruženju.

---

## 1. Pregled i filozofija dizajna

Novi interfejs zamenjuje vertikalno skrolujuću studio tabelu zaključanim, bezgraničnim (*borderless*) desktop radnim prostorom (`100vh × 100vw`, `overflow: hidden`) prilagođenim podrazumevanom prozoru **`1280×840`** (`minWidth: 960`, `minHeight: 640` u [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json)).

### Ključni vizuelni principi
1. **Duboka tamna noć (`#06070a`):** Pozadina celog canvas-a je skoro potpuno crna kako bi kontrast teksta, waveform prikaza i ambijentalnih odsjaja bio maksimalno čitljiv bez zamaranja očiju.
2. **Bez vidljivih ivica (`border: 0`):** Glavne forme, redovi u listi fajlova, polja za unos teksta i gornja granica timeline-a nemaju vidljive linije okvira (`border: 0`).
3. **Ambijentalni pozadinski odsjaj (`box-shadow`):** Umesto ivica, svaka forma lebdi na tamnoj pozadini definisana mekanim obojenim odsjajem koji se direktno poklapa sa bojom svoje trake na donjem timeline-u:
   - **Ubačeni fajlovi ↔ Traka 1 (`MP3`):** Ice Cyan (`#38bdf8`)
   - **Podešavanja:** Moonlight Silver (`#94a3b8` / `#e2e8f0`)
   - **Original transcript ↔ Traka 2 (`Original transcript`):** Cool Indigo (`#818cf8`)
   - **Prevod ↔ Traka 3 (`Prevod`):** Studio Emerald (`#10b981`)
   - **Završeni fajlovi (`.file-row.done`):** Zeleni smaragdni odsjaj i blagi gradijent bez spoljne ivice.

---

## 2. Dizajn sistem i CSS tokeni

### 2.1 Osnovna paleta i tipografija

| Token | Vrednost | Namena |
| :--- | :--- | :--- |
| `--bg-canvas` | `#06070a` | Glavna tamna pozadina canvas-a (`100vw × 100vh`) |
| `--bg-titlebar` | `#0a0c11` | Gornji taskbar (`#titlebar`) i donji footer (`#footer`) |
| `--bg-card` | `#090b12` | Površina glavnih formi (`#files-panel`, `#settings-panel`, `.editor-card`) |
| `--bg-card-head` | `#0c0f17` | Zaglavlja kartica (`36px`) |
| `--bg-well` | `#06080d` | Udubljena polja (`textarea`, `select`, `input`, `.s-toggle-card`) |
| `--bg-timeline` | `#080a0f` | Osnova timeline panela (`#timeline-panel`) |
| `--bg-timeline-head` | `#0b0e14` | Gornja traka timeline-a i levi panel sa nazivima traka |
| `--text-primary` | `#eef2f6` | Glavni tekst u editorima i aktivnim naslovima |
| `--text-title` | `#e6eaf2` | Naslov aplikacije i nazivi kartica/fajlova |
| `--text-secondary` | `#9aa5b4` | Sekundarne oznake i ghost dugmad |
| `--text-muted` | `#6e7788` | Footer, meta podaci i pomoćni tekstovi |
| `--font-sans` | `-apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, sans-serif` | UI tekstovi i tekst titlova |
| `--font-mono` | `ui-monospace, "Cascadia Mono", "JetBrains Mono", Consolas, monospace` | Vremenski kodovi, verzije, brojači, CPS i `font-variant-numeric: tabular-nums` |

### 2.2 Ambijentalni odsjaji (`box-shadow`) po panelima

```css
/* 1. Ubačeni fajlovi (#files-panel) — Cyan (#38bdf8) */
border: 0;
box-shadow:
  0 0 24px -2px rgba(56, 189, 248, 0.34),
  0 0 10px rgba(56, 189, 248, 0.18),
  inset 0 0 24px rgba(56, 189, 248, 0.05);

/* 1a. Završen fajl u listi (.file-row.done) — Zeleni odsjaj (#10b981) */
border: 0;
background: linear-gradient(
  90deg,
  rgba(16, 185, 129, 0.17) 0%,
  rgba(16, 185, 129, 0.07) 58%,
  rgba(16, 185, 129, 0.025) 100%
);
box-shadow:
  0 0 14px rgba(16, 185, 129, 0.2),
  inset 0 0 18px rgba(16, 185, 129, 0.1);
/* Aktivno selektovan završen fajl (.file-row.done.selected): 0 0 20px rgba(16, 185, 129, 0.34), inset 0 0 22px rgba(16, 185, 129, 0.16); */

/* 2. Podešavanja (#settings-panel) — Srebrni odsjaj */
border: 0;
box-shadow:
  0 0 24px -2px rgba(148, 163, 184, 0.26),
  0 0 10px rgba(226, 232, 240, 0.14),
  inset 0 0 22px rgba(148, 163, 184, 0.045);

/* 3. Original transcript (.editor-card.original) — Indigo (#818cf8) */
border: 0;
box-shadow:
  0 0 28px -2px rgba(129, 140, 248, 0.45),
  0 0 11px rgba(129, 140, 248, 0.24),
  inset 0 0 28px rgba(129, 140, 248, 0.07);
/* Na :focus-within: 0 0 34px 0px rgba(129, 140, 248, 0.58), 0 0 14px rgba(129, 140, 248, 0.34), inset 0 0 30px rgba(129, 140, 248, 0.1); */

/* 4. Prevod (.editor-card.prevod) — Smaragdni (#10b981) */
border: 0;
box-shadow:
  0 0 28px -2px rgba(16, 185, 129, 0.45),
  0 0 11px rgba(16, 185, 129, 0.24),
  inset 0 0 28px rgba(16, 185, 129, 0.07);
/* Na :focus-within: 0 0 34px 0px rgba(16, 185, 129, 0.58), 0 0 14px rgba(16, 185, 129, 0.34), inset 0 0 30px rgba(16, 185, 129, 0.1); */
```

> **Kritično CSS pravilo protiv odsecanja odsjaja:** Kontejneri `#canvas-stage` i `#editor-band` **moraju** imati `overflow: visible` sa dovoljnim unutrašnjim razmacima (`gap: 18px`, horizontalni `padding: 18px`), dok `#titlebar` i `#timeline-panel` imaju `position: relative; z-index: 2;`, a spoljni `.window-shell` i `#canvas` zadržavaju `overflow: hidden`. U suprotnom `box-shadow` odsjaj biva oštro presečen na ivicama grida.

---

## 3. Arhitektura rasporeda (`100vh` Desktop Shell)

```text
+------------------------------------------------------------------------------+
| [38px] #titlebar                  EasySRT Studio v0.1.0        [ _ ] [ □ ] [×]|
+--------------------------------------+---------------------------------------+
| [1fr] #canvas-stage (overflow: visible; gap: 18px; padding: 14px 18px 8px)   |
|  +--------------------------------+  +------------------------------------+  |
|  | #files-panel (Cyan odsjaj)     |  | #settings-panel (Srebrni odsjaj)   |  |
|  | Ubačeni fajlovi • + Dodaj fajl |  | [Obrada|Titlovi|Modeli|Cloud|...]  |  |
|  | [✓ zeleni odsjaj] Završeno     |  | Aktivni tab sadržaj                |  |
|  | [•] Obrada 64% / Na čekanju    |  |                                    |  |
|  +--------------------------------+  +------------------------------------+  |
+--------------------------------------+---------------------------------------+
| [264px] #editor-band (overflow: visible; gap: 18px; padding: 10px 18px 14px) |
|  +--------------------------------+  +------------------------------------+  |
|  | #card-original (Indigo odsjaj) |  | #card-prevod (Smaragdni odsjaj)    |  |
|  | Original transcript [EN]       |  | Prevod [SR]                        |  |
|  | <textarea> (border: 0)         |  | <textarea> (border: 0)             |  |
|  | zn. • CPS | Poništi | Sačuvaj  |  | zn. • CPS | Poništi | Sačuvaj      |  |
|  +--------------------------------+  +------------------------------------+  |
+------------------------------------------------------------------------------+
| [232px] #timeline-panel (border-top: 0; z-index: 2)                          |
|  [34px] Transport (Play/Stop/Timecode)                               Zoom    |
|  +---------------+--------------------------------------------------------+  |
|  | [24px] TIMELINE| 00:00   00:02   00:04   00:06 ... (klik/prevlačenje)  |  |
|  +---------------+--------------------------------------------------------+  |
|  | [58px] MP3     | ~~~~ HiDPI Waveform <canvas> ~~~~~~~~~~~~~~~~~~~~~~~~ |  |
|  | [58px] Original| [#1 ...]  [#2 ...]  [#3 ...]  ...                     |  |
|  | [58px] Prevod  | [#1 ...]  [#2 ...]  [#3 ...]  ...                     |  |
|  +---------------+--------------------------------------------------------+  |
+------------------------------------------------------------------------------+
| [30px] #footer       Besplatni alat koji izrađuje i održava IG-Systems • © 2026 |
+------------------------------------------------------------------------------+
```

### CSS Grid dimenzije (na `1280×840`)
- `.window-shell`: `grid-template-rows: 38px minmax(0, 1fr) 30px; height: 100vh; width: 100vw; overflow: hidden;`
- `#canvas`: `grid-template-rows: minmax(0, 1fr) 264px 232px;`
- `#canvas-stage`: `grid-template-columns: 1fr 1fr; gap: 18px; padding: 14px 18px 8px 18px;` (~`254px` visine kartice pri `1280×840`)
- `#editor-band`: `grid-template-columns: 1fr 1fr; gap: 18px; padding: 10px 18px 14px 18px; height: 264px;` (`240px` visine svake editor kartice)
- `#timeline-panel`: `height: 232px; grid-template-rows: 34px 198px;` (`24px` lenjir + `3 × 58px` trake; leva kolona `184px`)

---

## 4. Specifikacija komponenti i mapiranje na Zustand / Tauri

Sve komponente komuniciraju sa postojećim store-om [`useAppStore`](../src/stores/appStore.ts) i tipovima iz [`src/types/models.ts`](../src/types/models.ts).

### 4.1 `WindowTitleBar.tsx` (`#titlebar`, `38px`)
- **Raspored:** `grid-template-columns: 1fr auto 1fr` bez asimetričnog levog `padding`-a, čime su `EasySRT Studio` (`12.5px`, `font-weight: 600`) i `v0.1.0` (`11.5px` mono) matematički centrirani u sredini prozora.
- **Tauri prevlačenje prozora:** Atribut `data-tauri-drag-region` na `#titlebar` i `.titlebar-center` (`pointer-events: none` na tekstu), dok dugmad za kontrolu prozora nemaju drag region.
- **Kontrole desno (`46×38px` po dugmetu):**
  - Minimize (`#btn-minimize`), Maximize/Restore (`#btn-maximize`, menja SVG ikonicu prema `win.isMaximized()`), Close (`#btn-close`, crveni hover `rgba(220, 38, 38, 0.9)`).
  - Poziva `getCurrentWindow()` iz `@tauri-apps/api/window` uz obaveznu proveru `isTauriRuntime()` iz [`src/lib/subtitleUtils.ts`](../src/lib/subtitleUtils.ts).
- **Potrebne izmene u Tauri konfiguraciji pri implementaciji:**
  1. [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json) → u `app.windows[0]` dodati `"decorations": false`.
  2. [`src-tauri/capabilities/default.json`](../src-tauri/capabilities/default.json) → u `permissions` dodati:
     - `"core:window:allow-minimize"`
     - `"core:window:allow-toggle-maximize"`
     - `"core:window:allow-close"`
     - `"core:window:allow-start-dragging"`
     - `"core:window:allow-is-maximized"`

### 4.2 `FilesPanel.tsx` (`#files-panel`, gore levo)
- **Zaglavlje (`36px`):** ikonica dokumenta (`#38bdf8`), naslov **Ubačeni fajlovi**, pilula `${completedCount}/${batchQueue.length} završeno`, i dugme **+ Dodaj fajl**.
- **Unos fajlova:**
  - Klik na **+ Dodaj fajl** koristi `@tauri-apps/plugin-dialog` `open({ multiple: true })` u Tauri režimu (audio + `.srt`/`.vtt`), odnosno skriveni `<input type="file" multiple>` u browser fallback-u.
  - Drag-and-drop preko `#files-panel` (aktivira `.dragover` pojačani cijan odsjaj) razdvaja audio/video fajlove (`enqueueAudioFiles`) i postojeće titlove (`importSubtitleFromPath` / `importSubtitleFromText`).
- **Mapiranje `BatchQueueItem` (`batchQueue`) i `JobPhase` statusa:**
  - `phase === "completed"` → klasa `.file-row.done` (zeleni pozadinski odsjaj, svetleća smaragdna ikonica kvačice, bedž **Završeno**, meta `FORMAT • trajanje • N titlova`).
  - Aktivna faza (`convertingAudio`, `transcribing`, `segmenting`, `translating`, `polishing`) → klasa `.file-row.processing`, inline cijan/smaragdni progress bar (`item.percent`) i bedž `Obrada XX%`, plus dugme za prekid (`cancelJobById(item.jobId)`).
  - `queued` → neutralan red sa bedžom **Na čekanju**.
  - `failed` / `cancelled` → crveni/neutralan bedž greške sa `item.errorMessage` u tooltip-u.
- **Interakcija:**
  - Klik na red poziva `selectQueueItem(item.jobId)` (ažurira `activeJobId`, `activeProject` i naziv na MP3 traci timeline-a).
  - Hover prikazuje diskretno `×` dugme (`removeQueueItem(item.jobId)`).
  - **Pokretanje obrade i sačuvane sesije (Checkpointi):** Prilikom implementacije omogućiti pokretanje `startBatchPipeline("full")` / `startBatchPipeline("transcribeOnly")` i učitavanje prethodnih sesija (`checkpoints`, `loadCheckpointIntoEditor`) putem kompaktnih akcija u zaglavlju/podnožju `#files-panel` (ili kao opcija u tabu `Obrada`), bez narušavanja bezgranične siluete panela.

### 4.3 `SettingsTabsPanel.tsx` (`#settings-panel`, gore desno)
- **Zaglavlje (`36px`):** 6 tabova u jednom redu (`Obrada`, `Titlovi`, `Modeli`, `Cloud API`, `Izvoz`, `Opšte`) sa donjom `#38bdf8` linijom aktivnog taba + desni `#settings-mode-pill` (`OFFLINE` zeleni ili `ONLINE • DeepSeek` / `ONLINE • Custom` narandžasti).
- **Mapiranje tabova 1:1 na `useAppStore`:**

| Tab | Kontrole u tabu | Zustand stanje & akcije |
| :--- | :--- | :--- |
| **1. Obrada** | `Izvorni jezik (Audio)`, `Ciljni jezik (Prevod)`, `Srpsko pismo` (`Latinica \| Ćirilica`), `LLM poliranje` prekidač, `Pojmovnik (Glossary)` čipovi + inline unos | `jobOptions.sourceLanguage`, `jobOptions.targetLanguage`, `jobOptions.polishSerbian`, `jobOptions.glossary` preko `setJobOptions(...)`; `jobOptions.script` preko `convertActiveProjectScript(script)` |
| **2. Titlovi** | `Profil segmentacije` (`Standardni 2×42 \| Reels 1×20 \| Prilagođeno`), `Maks. znakova`, `Maks. redova`, `Maks. CPS`, `Maks. trajanje (s)`, dugme `Primeni na titlove` | `jobOptions.segmentRules` preko `setSegmentPreset(...)`, `updateSegmentRules(...)` i `applyResegmentToActiveProject()` |
| **3. Modeli** | GPU status baner (`setupStatus.gpu` + `readyForOffline`), izbor aktivnog Whisper i LLM modela, `modelsDirOverride` (`Izaberi...`), `+ Uvezi .bin / .gguf` + otvaranje čarobnjaka | `setupStatus`, `catalog`, `config.activeWhisperModelId`, `config.activeLlmModelId`, `config.modelsDirOverride` (`updateConfig`), `runImportCustomModel`, `setSetupOpen(true)` |
| **4. Cloud API** | `Pogon za prevod` (`Lokalni LLM \| DeepSeek API \| Custom OpenAI`), `API ključ` (`Sačuvaj`), `Model / Endpoint`, `batchSize`, `contextWindow` | `setEffectiveProvider(provider)`, `apiKeyStatus`, `runSetApiKey`, `runClearApiKey`, `config.translation` (`updateConfig`) |
| **5. Izvoz** | Čipovi formata (`.SRT`, `.VTT`, `.TXT`), prekidači `Izvezi i izvorni .SRT` i `UTF-8 sa BOM-om`, `Lokacija čuvanja` (`Pored audio fajla \| Prilagođeni folder`), putanja + dugme za izvoz | `config.export` (`updateConfig`), `runExportSubtitles(config.export)` i `quickExportSrt()` |
| **6. Opšte** | `Jezik interfejsa` (`Srpski \| English`), `Podrazumevano pismo`, prikaz režima skladištenja (`setupStatus.storage.mode`), automatski checkpoint-i, `Proveri ažuriranja` | `setUiLanguage(lang)`, `config.defaultScript` (`updateConfig`), `setupStatus.storage`, `setUpdaterOpen(true)` |

### 4.4 `DualSubtitleEditors.tsx` (`#editor-band`, `264px`)
- **Dve uparene kartice:**
  - Leva kartica (`#card-original`, indigo `#818cf8` odsjaj): **Original transcript** `[EN]`, povezana na `cue.sourceText`.
  - Desna kartica (`#card-prevod`, smaragdni `#10b981` odsjaj): **Prevod** `[SR]`, povezana na `cue.translatedText`.
- **Sinhronizacija sa aktivnim titlom:** Prikazuje aktivni segment iz `activeProject?.cues.find(c => c.id === activeCueId)` sa zaglavljem `#ID • HH:MM:SS,mmm → HH:MM:SS,mmm` (`formatTimestampSrt` iz [`src/lib/subtitleUtils.ts`](../src/lib/subtitleUtils.ts)).
- **Lokalni draft i čuvanje izmena:**
  - Kucanje menja lokalni draft, prebacuje pilulu u **Nesačuvano** (amber tačka) i aktivira dugmad **Poništi** i **Sačuvaj izmenu**.
  - Klik na **Sačuvaj izmenu** (ili `Ctrl+S` kada je fokus u `textarea`) poziva `updateCue(cue.id, { sourceText | translatedText })`, što automatski čuva undo snimak, preračunava CPS/upozorenja, osvežava blok na timeline-u i prikazuje privremeni status `✓ Sačuvano` (2s).
- **Žive metrike:** Donji levi ugao svake kartice računa `zn. • CPS` u realnom vremenu; prekoračenje `jobOptions.segmentRules.maxCps` boji vrednost u amber (`#fbbf24`), a `maxCps + 4` u rose (`#fb7185`).
- **Prazno stanje:** Kada je `activeProject === null`, obe kartice prikazuju onemogućeno polje sa suptilnom porukom da se izabere ili uveze fajl.

### 4.5 `TimelinePanel.tsx` (`#timeline-panel`, `232px`)
- **Gornja kontrolna traka (`34px`):**
  - Levo: `Play/Pause`, `Stop` (vraća playhead na `0`), tabular timecode `trenutno / ukupno`.
  - Desno: Zoom kontrole (`−`, slider `100%–300%`, procenti, `+`).
- **Leva fiksna kolona (`184px`):**
  - Zaglavlje lenjira: `TIMELINE` (`24px`).
  - **Traka 1 — `MP3` (`Audio`, `#38bdf8`):** tasteri `M` (Mute — mutira HTML `<audio>`) i `Lock`.
  - **Traka 2 — `Original transcript` (`EN`, `#818cf8`):** tasteri za vidljivost (oko) i `Lock` (zaključava izmenu `sourceText`).
  - **Traka 3 — `Prevod` (`SR`, `#10b981`):** tasteri za vidljivost (oko) i `Lock` (zaključava izmenu `translatedText`).
- **Desni skrolujući viewport (`TOTAL_MS`):**
  - **Lenjir (`24px`):** dinamički koraci (`2s`/`1s` prema zoom nivou). Klik i prevlačenje po lenjiru pomera playhead (`scrubbing`) i `audioRef.current.currentTime`.
  - **Traka 1 (`58px`):** kontinuirani MP3 klip sa HiDPI `<canvas id="mp3-waveform">`. Za prave audio fajlove u Tauri režimu (`convertFileSrc(activeProject.audioPath)`) dekodira stvarne pikove putem `AudioContext.decodeAudioData` (keširano po `audioPath`), a za `.srt`/`.vtt` ili browser fallback koristi deterministički prikaz govornih segmenata.
  - **Trake 2 i 3 (`58px` svaka):** blokovi svih titlova iz `activeProject.cues` pozicionirani proporcionalno (`startMs` → `endMs`).
    - Jedan klik selektuje titl (`setActiveCueId(cue.id)`) i učitava ga u oba editora iznad.
    - Dupli klik pomera playhead na `cue.startMs` i pokreće preslušavanje segmenta (`playCueSegment(cue)`).
  - **Vertikalni playhead:** proteže se od vrha lenjira do dna treće trake i prati `<audio>` putem `requestAnimationFrame`.

### 4.6 `AppFooter.tsx` (`#footer`, `30px`)
- Centriran tekst: `Besplatni alat koji izrađuje i održava IG-Systems • © 2026`.

---

## 5. Modali, notifikacije i prečice na tastaturi

- **Zadržani modali iz postojećeg koda:**
  - [`SetupWizard.tsx`](../src/components/SetupWizard.tsx) — otvara se automatski pri prvom pokretanju kada nedostaju lokalne komponente, ili iz taba `Modeli`.
  - [`OnlineConfirmModal.tsx`](../src/components/OnlineConfirmModal.tsx) — obavezna potvrda privatnosti pre slanja teksta na DeepSeek / Custom OpenAI API.
  - [`UpdaterModal.tsx`](../src/components/UpdaterModal.tsx) — otvara se na dugme `Proveri ažuriranja` u tabu `Opšte`.
- **Plutajući baner (`banner`):** Prikazuje se kao `position: fixed` *toast* u gornjem centru ispod `#titlebar` (`top: 46px; z-index: 40;`) kako nikada ne bi pomerio `100vh` visinu radnog prostora.
- **Prečice na tastaturi:**
  - `Space` — Play / Pause na timeline-u (**obavezno ignoriše** fokus unutar `INPUT`, `TEXTAREA`, `SELECT` i `BUTTON`).
  - `Ctrl+S` / `Cmd+S` — čuva izmene u trenutno fokusiranom editoru (`Original transcript` ili `Prevod`).
  - `Ctrl+Z` / `Ctrl+Y` (van tekstualnog polja) — `undoCueEdit()` / `redoCueEdit()`.
  - `Alt+ArrowUp` / `Alt+ArrowDown` — prelazak na prethodni / sledeći titl i fokusiranje na timeline-u.

---

## 6. Mapa migracije postojećih fajlova

| Postojeći fajl | Status u sledećem koraku | Napomena |
| :--- | :--- | :--- |
| [`src/App.tsx`](../src/App.tsx) | **Redizajn** | Prebacuje se na `.window-shell` (`38px \| 1fr \| 30px`) sa `WindowTitleBar`, `FilesPanel`, `SettingsTabsPanel`, `DualSubtitleEditors`, `TimelinePanel`, `AppFooter` i postojećim modalima |
| [`src/index.css`](../src/index.css) | **Ažuriranje** | Dodaje dark-night CSS tokene, borderless glow klase i slim scrollbar stilove |
| [`src/components/TopBar.tsx`](../src/components/TopBar.tsx) | **Zamena** | Zamenjuje ga `WindowTitleBar.tsx` + status pilula unutar `SettingsTabsPanel` |
| [`src/components/BatchQueuePanel.tsx`](../src/components/BatchQueuePanel.tsx) | **Zamena** | Integrisano u `FilesPanel.tsx` sa zelenim odsjajem završenih fajlova |
| [`src/components/JobWorkspace.tsx`](../src/components/JobWorkspace.tsx) | **Zamena** | Opcije premeštene u `SettingsTabsPanel.tsx` (6 tabova), upload u `FilesPanel.tsx` |
| [`src/components/SubtitleTable.tsx`](../src/components/SubtitleTable.tsx) | **Zamena** | Zamenjeno parom `DualSubtitleEditors.tsx` + `TimelinePanel.tsx` (trake 2 i 3) |
| [`src/components/AudioSegmentPlayer.tsx`](../src/components/AudioSegmentPlayer.tsx) | **Zamena** | Integrisano u `TimelinePanel.tsx` (transport + `<audio>` + HiDPI waveform) |
| [`src/components/SettingsModal.tsx`](../src/components/SettingsModal.tsx) | **Konsolidacija** | Sadržaj pokriven tabovima `Modeli`, `Cloud API`, `Opšte` u `SettingsTabsPanel.tsx` |
| [`src/components/ExportModal.tsx`](../src/components/ExportModal.tsx) | **Konsolidacija** | Sadržaj pokriven tabom `Izvoz` u `SettingsTabsPanel.tsx` (ili opcionalni brzi dijalog) |
| [`src/components/SetupWizard.tsx`](../src/components/SetupWizard.tsx), [`OnlineConfirmModal.tsx`](../src/components/OnlineConfirmModal.tsx), [`UpdaterModal.tsx`](../src/components/UpdaterModal.tsx) | **Zadržano** | Prilagoditi im pozadine/senke na `#090b12` tamni noćni stil bez oštrih okvira |
| [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json), [`src-tauri/capabilities/default.json`](../src-tauri/capabilities/default.json) | **Dopuna** | `"decorations": false` + window capability dozvole za minimize/maximize/close/drag |

---

## 7. Kriterijumi prihvatanja i provera nakon implementacije

Nakon implementacije u React + Tauri kodu, svaka promena mora proći sledeće provere:

1. **Statička i build verifikacija:**
   ```bash
   npm run typecheck && npm test && npm run build
   cd src-tauri && cargo check --all-targets && cargo test
   ```
2. **Vizuelna i interaktivna provera (`1280×840` i `960×640`):**
   - `0` konzolnih grešaka, `0` horizontalnog skrola (`scrollWidth <= innerWidth`).
   - Sve 4 gornje/srednje forme i redovi u listi fajlova nemaju spoljne ivice (`border: 0`) i imaju čist, neisečen pozadinski odsjaj u boji.
   - Svih 6 tabova u panelu podešavanja staju u zaglavlje bez prelamanja i ne izlaze iz kontejnera.
   - Klik na blok u timeline-u sinhronizuje oba editora; izmena i čuvanje odmah ažurira tekst u traci i `useAppStore`; prebacivanje `Latinica ↔ Ćirilica` deterministički preslovljava titlove bez kvarenja tehničkih termina; `Space` unutar `textarea` unosi razmak umesto da pokreće plejer.
