🌐 [English](../en/troubleshooting.md) | **Srpski** · [← Nazad na README](../../../README.sr.md) · [Korisnički vodič](guide.md)

# EasySRT Studio — Rešavanje problema i FAQ

Brza rešenja za GPU ubrzanje, memoriju lokalnih AI modela, bezbednost API ključeva, reprodukciju zvuka i prijavu grešaka.

---

## 1. GPU ubrzanje (Windows CUDA i Linux Vulkan)

- **Windows NVIDIA grafička kartica radi preko procesora (CPU) umesto CUDA ubrzanja:**
  - EasySRT Studio već sadrži sve potrebne CUDA 12.4 biblioteke — **nije** potrebno instalirati veliki NVIDIA CUDA Toolkit, već samo noviji **NVIDIA Game Ready ili Studio drajver (`>= 550.54`)**.
  - Ažurirajte NVIDIA drajver, ponovo pokrenite EasySRT Studio i u **Podešavanjima (Settings)** proverite da li je prepoznat vaš VRAM nivo.
- **Linux GPU ubrzanje prijavljuje nedostatak biblioteka (`libvulkan.so.1`):**
  - Instalirajte sistemski Vulkan paket vaše distribucije (npr. `sudo apt install libvulkan1 mesa-vulkan-drivers` na Ubuntu/Debian sistemima ili `vulkan-icd-loader` na Arch Linux-u).
  - Čak i ako GPU binarni fajl ne može da se pokrene, EasySRT Studio automatski prelazi na ispravan CPU režim kako se transkripcija ne bi prekinula.

---

## 2. Memorija lokalnih AI modela (VRAM) i brzina rada

- **Laptop grafičke kartice sa 4 GB VRAM-a (npr. RTX 3050 4 GB) ostaju bez memorije:**
  - U Čarobnjaku za podešavanje izaberite **`Gemma 3 4B IT (Q4_K_M)`** ili **`Qwen 3 4B (Q4_K_M)`**.
  - EasySRT Studio automatski pokreće `llama-server` sa kvantizovanim KV kešom i prilagođavanjem slojeva (`-fitc 2048 -ctk q8_0 -ctv q8_0`). Ako drugi zahtevni programi zauzimaju grafičku karticu, zatvorite ih ili pustite program da automatski pređe na CPU (`-ngl 0`).
- **Spora oflajn obrada na računarima bez zasebne grafičke kartice:**
  - Prebacite se na **`Hybrid Turbo (GPU/CPU + DeepSeek API)`** za trenutno prevođenje u oblaku ili izaberite **`Whisper Medium (Q5_0)`** za bržu lokalnu CPU transkripciju.

---

## 3. Cloud API ključevi i sopstveni OpenAI-kompatibilni serveri

- **Gde se čuvaju moji API ključevi?**
  - Ključevi se prvenstveno čuvaju u sistemskom trezoru operativnog sistema (**Windows Credential Manager**, **Linux Keyring** ili **macOS Keychain**).
  - Ako sistemski trezor nije dostupan u vašem okruženju, ključ ostaje samo u radnoj memoriji (RAM) tokom sesije, osim ako izričito ne uključite šifrovani lokalni trezor (`.credentials.json` zaštićen pomoću Windows DPAPI-ja ili lokalne autentifikovane enkripcije).
- **Zašto je moj prilagođeni (Custom OpenAI) URL odbijen?**
  - Radi bezbednosti (zaštita od SSRF napada i curenja ključeva), udaljeni serveri moraju koristiti **`https://`** (lokalne adrese `http://127.0.0.1` i `localhost` su dozvoljene za alate poput Ollama ili LM Studio), ne smeju sadržati lozinke u samom URL-u (`user:pass@host`) i ne prate preusmeravanja na druge domene.

---

## 4. Reprodukcija zvuka, zvučni talas i oporavak sesije

- **Zvučni talas ili reprodukcija ne kreću na Linux-u ili kod određenih video formata:**
  - Proverite da li je instaliran sistemski `ffmpeg`. EasySRT Studio strimuje medije direktno bez opterećenja memorije za `.mp3`, `.wav`, `.m4a`, `.flac`, `.ogg`, `.aac`, `.opus`, `.mp4`, `.mkv` i `.webm` fajlove.
- **Šta se dešava ako se program zatvori ili računar restartuje usred posla?**
  - Svaka transkripcija i svaka prevedena grupa titlova automatski se čuvaju u lokalnom folderu `checkpoints/` (dok se ručne izmene u editoru čuvaju posle `600 ms`). Ponovo otvorite program i nastavite rad iz pregleda sačuvanih projekata bez ponavljanja već obrađenih delova.

---

## 5. Prijava grešaka i garancija privatnosti

- **Kako da prijavim grešku ili pošaljem predlog za novu funkciju?**
  - Kliknite na dugme za prijavu greške / predloga u donjem podnožju (footeru) aplikacije.
- **Da li se moji audio snimci ili tekstovi titlova šalju uz izveštaj o grešci?**
  - **Nikada.** Dijagnostički modul automatski uklanja korisničko ime računara, lokalne putanje, API ključeve i tokene, i nikada ne prilaže vaše snimke niti tekst titlova. Ako radite bez interneta, izveštaj možete sačuvati lokalno i sami pregledati pre slanja.
