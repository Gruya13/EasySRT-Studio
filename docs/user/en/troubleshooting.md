🌐 **English** | [Srpski](../sr/troubleshooting.md) · [← Back to README](../../../README.md) · [User Guide](guide.md)

# EasySRT Studio — Troubleshooting & FAQ

Quick solutions for GPU acceleration, local AI memory limits, API key security, media playback, and crash diagnostics.

---

## 1. GPU Acceleration (Windows CUDA & Linux Vulkan)

- **Windows NVIDIA GPU runs on CPU instead of CUDA:**
  - EasySRT Studio bundles CUDA 12.4 runtime libraries automatically — you do **not** need the multi-gigabyte NVIDIA CUDA Toolkit, only a modern **NVIDIA Game Ready or Studio Driver (`>= 550.54`)**.
  - Update your NVIDIA display driver, restart EasySRT Studio, and check **Settings → System / Hardware** to confirm your VRAM tier is detected.
- **Linux GPU acceleration fails with missing shared libraries (`libvulkan.so.1`):**
  - Install your distribution's Vulkan loader package (for example `sudo apt install libvulkan1 mesa-vulkan-drivers` on Ubuntu/Debian or `vulkan-icd-loader` on Arch).
  - If a GPU binary still cannot launch, EasySRT Studio's pre-flight check automatically falls back to a runnable CPU binary so your transcription job still completes safely.

---

## 2. Offline Local AI Memory (VRAM) & Speed

- **4 GB Laptop GPUs (e.g. RTX 3050 4 GB) running out of VRAM:**
  - Select **`Gemma 3 4B IT (Q4_K_M)`** or **`Qwen 3 4B (Q4_K_M)`** in the Setup Wizard.
  - EasySRT Studio automatically launches `llama-server` with quantized KV cache and layer auto-fitting (`-fitc 2048 -ctk q8_0 -ctv q8_0`). If another heavy app is consuming GPU memory, close GPU-intensive apps or let EasySRT Studio fall back to CPU (`-ngl 0`) automatically.
- **Slow offline processing on CPU-only computers:**
  - Switch to **`Hybrid Turbo (GPU/CPU + DeepSeek API)`** for instant cloud translation, or choose **`Whisper Medium (Q5_0)`** when transcribing on machines without a dedicated GPU.

---

## 3. Cloud API Keys & Custom OpenAI Endpoints

- **Where are my API keys stored?**
  - Keys are stored primarily in your operating system's native credential manager (**Windows Credential Manager**, **Linux Keyring**, or **macOS Keychain**).
  - If an OS keyring is unavailable in your environment, keys stay in RAM for the current session unless you explicitly opt in to the encrypted local vault (`.credentials.json` protected via Windows DPAPI or authenticated local encryption).
- **My Custom OpenAI-compatible URL was rejected:**
  - For security (SSRF and credential protection), remote endpoints must use **`https://`** (loopback `http://127.0.0.1` / `localhost` is permitted for local servers like Ollama or LM Studio), cannot embed credentials in the URL (`user:pass@host`), and do not follow cross-host HTTP redirects.

---

## 4. Audio Playback, Waveforms & Session Recovery

- **Waveform or audio playback does not start or triggers unexpected background resume:**
  - Ensure system `ffmpeg` is installed. EasySRT Studio uses lazy `<audio>` attachment, clears `navigator.mediaSession` when paused (`MPRIS` / Windows `SMTC` isolation), and suspends its Web `AudioContext` on pause so external media keys or dictation tools cannot resume background audio unexpectedly.
- **Duplicate titlebars on Windows 10/11:**
  - Studio uses a frameless custom titlebar (`decorations: false`) maximized into the OS taskbar work area cleanly.
- **What happens if the app closes or my PC restarts mid-job?**
  - Every transcription and every translated batch is saved atomically to your local `checkpoints/` folder (and manual cue edits autosave after `600 ms`). Reopen EasySRT Studio and resume your project from the **Checkpoints / History** view without re-running finished batches.

---

## 5. Reporting Bugs & Privacy Guarantees

- **How do I report a problem or request a feature?**
  - Click the **Bug / Feedback** button in the bottom footer of the app.
- **Are my audio files or subtitle texts ever included in bug reports?**
  - **Never.** Diagnostics automatically redact local usernames, file paths, API keys, and bearer tokens, and never attach your media recordings or subtitle content. If you are offline, you can also export the sanitized diagnostic report locally to inspect every line yourself before sharing.
