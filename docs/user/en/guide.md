🌐 **English** | [Srpski](../sr/guide.md) · [← Back to README](../../../README.md) · [Troubleshooting & FAQ](troubleshooting.md)

# EasySRT Studio — User Guide

Everything you need to install EasySRT Studio, choose the right AI workflow for your hardware, edit subtitles on the interactive waveform timeline, and export clean broadcast files.

---

## 1. Installation & First Launch

### Windows 10 / 11 (x64)
1. Download **`EasySRT.Studio_<version>_x64-setup.exe`** from [GitHub Releases](https://github.com/Gruya13/EasySRT-Studio/releases/latest) and run the installer.
2. During setup, EasySRT Studio automatically provisions FFmpeg, hardware-matched Whisper speech binaries (CUDA 12.4 GPU or CPU), Silero VAD, and the default **Whisper Large V3 Turbo (Q5_0)** transcription model.
3. On first launch, the **Setup Wizard** detects your GPU VRAM and lets you either download a recommended free local translation model or connect a Cloud API key.

### Linux (x64)
1. Download **`EasySRT.Studio_<version>_amd64.AppImage`**, mark it executable (`chmod +x EasySRT.Studio_*.AppImage`), and launch it.
2. Ensure `ffmpeg` is installed via your package manager (or install components directly inside **Settings → Setup Wizard**).
3. For cross-vendor GPU acceleration (NVIDIA, AMD, Intel) on Linux, you can select Vulkan runtime binaries in the Setup Wizard or extract the optional `whisper-bin-ubuntu-vulkan-x64.tar.gz` release archive.

---

## 2. Workflow Modes & Hardware Recommendations

Switch modes anytime from the **Settings** panel (`Processing` / `Cloud API`):

| Workflow Mode | Best For | Recommended Setup |
| :--- | :--- | :--- |
| **100% Offline (Local AI)** | Confidential client recordings, NDA interviews, zero recurring API cost | **4 GB VRAM:** `Gemma 3 4B (Q4_K_M)` or `Qwen 3 4B`<br>**8 GB VRAM:** `Gemma 4 E4B (QAT Q4_0)` or `Gemma 3 4B (Q8_0)`<br>**12 GB+ VRAM:** `Gemma 4 12B` or `Gemma 3 12B` |
| **Hybrid Turbo (GPU + Cloud API)** | Fast multi-file batches (up to **28x real-time**) & highest idiomatic quality | Local GPU Whisper (`Large V3 Turbo Q5_0`) + **DeepSeek API** (`deepseek-chat`) or any OpenAI-compatible HTTPS endpoint |
| **Transcribe Only (No LLM)** | Same-language captions when no translation or LLM polishing is needed | `Whisper Large V3 Turbo (Q5_0)` on GPU, or `Whisper Medium (Q5_0)` on CPU-only machines |

- **Sequential VRAM Protection:** Offline mode never loads Whisper and the local LLM into VRAM at the same time — Whisper finishes and releases 100% of GPU memory before `llama-server` starts.
- **Smart Multi-File Batches:** Offline multi-file queues transcribe all files first and then reuse a single warm local LLM server; Cloud multi-file queues overlap GPU transcription of file `N+1` with cloud translation of file `N`.

---

## 3. Importing Media, Smart Intake Probe & Segmentation Rules

- **Supported Media Formats:** `.mp3`, `.wav`, `.m4a`, `.flac`, `.ogg`, `.aac`, `.opus`, `.mp4`, `.mkv`, `.webm`.
- **Direct Subtitle Import:** Drag in existing `.srt` or `.vtt` files to translate, polish Serbian grammar, convert scripts, condense reading speed, or adjust timing on the timeline.
- **Background Media Intake Probe:** As soon as you drop an audio or video file into the queue, EasySRT Studio quietly analyzes speaker count, spoken language, and topic context in the background (and pauses immediately the moment you click **Run** so it never slows down real jobs).
- **Formatting & Segmentation Controls:**
  - **Presets:** `Standard` (`2 × 42` chars for TV/Film/YouTube), `Reels / Shorts` (`1 × 20` punchy captions), or `Custom` (`1–4` balanced lines and custom character/CPS limits).
  - **Speaker-Turn Split:** Automatically starts a new subtitle cue whenever the acoustic diarizer detects a change of speaker.
  - **Clean Spoken Fillers & Comma Splitting:** Optionally strips hesitation fillers (*uh, um, ovaj, znači…*), keeps leading conjunctions attached to the clause that follows, and splits long sentences naturally at commas.
  - **Source ASR Proofreading (`correctSourceAsr`):** Fixes obvious phonetic speech-to-text slips in the source column during translation without altering speaker wording (**enabled automatically when a Cloud API provider such as DeepSeek or Custom OpenAI is active**).
  - **Glossary Hints & Pre-Translation Domain Mini-Glossary:** Add domain names, brands, or `Source => Target` pairs so both Whisper (prompted with detected topic context) and the translator respect exact terminology, complemented automatically by a pre-translation domain mini-glossary extracted right before batch 1.

---

## 4. Cinema Dual-Pane Editor, Timeline & Keyboard Shortcuts

The workspace combines a side-by-side **Source (`Original`)** and **Translation (`Prevod`)** editor with independent font-size controls, `600 ms` automatic checkpoint saving, and a 3-track streaming **Waveform Timeline**:

- **Follow Playhead (`Prati plejhed`) & Gap-Hold Selection:** Enabled by default in the timeline toolbar, **Follow Playhead** keeps the viewport auto-scrolling when zoomed in (`100%–2000%`) and holds the nearest subtitle selected across inter-cue breathing pauses so editor boxes never flicker to *"No cue selected"*.
- **Exclusive `Space` Transport:** Pressing `Space` / `Shift+Space` outside text inputs always controls timeline playback (even if a toolbar button was focused), while yielding cleanly whenever a modal dialog is open.

| Shortcut / Action | Effect |
| :--- | :--- |
| `Space` | Play / Pause audio at the current playhead marker |
| `Shift + Space` | Play from the very beginning (`00:00.000`) |
| `←` / `→` | Step playhead backward / forward by 1 frame (`40 ms`) |
| `Shift + ←` / `Shift + →` | Jump playhead backward / forward by `1 second` |
| `↑` / `↓` | Jump to and play the next / previous subtitle cue |
| `C` | Split the subtitle cue under the playhead marker (leaves a clean `80 ms` breathing gap) |
| `Alt + 1` / `Alt + 2` | Focus the **Original** (`Alt+1`) or **Translation** (`Alt+2`) text box for the active cue |
| `Ctrl + S` / `Ctrl + Enter` | Save edits in the active text box (`Ctrl + Enter` also returns focus to the timeline) |
| `Esc` | Exit text box focus back to timeline shortcuts |
| `Ctrl + Z` / `Ctrl + Y` | Undo / Redo cue text and timeline edits (`Ctrl + Shift + Z` also redoes) |
| **Mouse Wheel on Timeline** | Smooth cursor-anchored zoom from **100% up to 2000%** |
| **Drag Cue Left / Right Handles** | Trim exact subtitle start and end timestamps directly against the audio waveform |

---

## 5. Serbian Studio Features & Exporting

- **Decoupled Style & Address Register (`Auto` / `Polite Vi` / `Informal ti`):** Choose `Natural` vs. `Formal` linguistic tone independently from polite `Vi` vs. informal `ti` addressing (default `Auto` selects `Vi` for press/TV interviews and `ti` for casual peer podcasts), plus anti-calque rules and Serbian conjunction-comma cleanup (`I,`, `Tako da,`).
- **Instant `Natural` ↔ `Formal` Style Switch (`0 ms`):** Compare or switch between living spoken Serbian (`Natural`) and broadcast/official phrasing (`Formal`). Cached dual-style variants swap instantaneously while keeping independent manual edits per style.
- **Ekavian Standard & Deterministic `Latin ↔ Cyrillic`:** Built-in linguistic normalization enforces clean Ekavian forms and converts between Latin and Cyrillic with proper digraph casing (`Ljubav` ↔ `Љубав`, `LJUBAV` ↔ `ЉУБАВ`), prefix-boundary exceptions (`injekcija` → `инјекција`, `konjugacija` → `конјугација`, `Tanjug` → `Танјуг`), and untouched HTML/ASS tags and URLs.
- **1-Click Auto-Fit CPS:** Tightens only cues flagged for excessive reading speed (`high_cps` / `line_too_long`) while leaving compliant subtitles untouched.
- **Multi-Format Export:** Export **`.SRT`**, **`.VTT`**, or plain **`.TXT`** transcripts, with optional **UTF-8 BOM** (for legacy Windows NLE editors) and optional dual export (translated `.srt` + source `.srt` in one pass).
