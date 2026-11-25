<div align="center">

🌐 [English](README.md) | **Srpski**

# EasySRT Studio

**Brz i privatan AI studio za titlove namenjen video montažerima, podkasterima i kreatorima**

*Pretvorite bilo koji audio ili video snimak u čiste, televizijski spremne titlove u dvostrukom Cinema editoru — uz instalaciju u jednom koraku, 100% oflajn privatnost ili hibridnu brzinu do **28x brže od realnog vremena**.*

[![Izdanje v0.4.0](https://img.shields.io/badge/izdanje-v0.4.0-38bdf8?style=for-the-badge&logo=github)](https://github.com/Gruya13/EasySRT-Studio/releases/latest)
[![Licenca: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-10b981?style=for-the-badge)](LICENSE)
[![Platforme](https://img.shields.io/badge/platform-Windows%2010%2F11%20%7C%20Linux-6366f1?style=for-the-badge)](https://github.com/Gruya13/EasySRT-Studio/releases/latest)

### [Preuzmi EasySRT Studio v0.4.0](https://github.com/Gruya13/EasySRT-Studio/releases/latest)

[Korisnički vodič](docs/user/sr/guide.md) · [Rešavanje problema i FAQ](docs/user/sr/troubleshooting.md) · [Istorija izdanja](CHANGELOG.md)

</div>

---

## Zašto kreatori biraju EasySRT Studio

- **Spremno za rad bez ručnog podešavanja**: Windows instaler automatski postavlja FFmpeg, GPU ubrzanje i prepoznavanje govora u jednom koraku — pokrenite program, izaberite besplatan lokalni AI prevodilac ili unesite Cloud API ključ i odmah krenite sa radom.
- **Cinema dvostruki editor i 2000% striming tajmlajn**: Uređujte izvor i prevod paralelno uz nezavisnu veličinu slova i automatsko čuvanje napretka, preslušavajte audio talas bez čekanja, pomerajte ivice titlova i zumirajte točkićem miša do **2000%**.
- **Prirodan srpski jezik (`Prirodni` i `Formalni` stil za 0 ms)**: Trenutno prebacujte između živog govornog i formalnog ekavskog stila bez ponovnog prevođenja, automatski uklonite poštapalice, skratite prebrze titlove jednim klikom (**Auto-Fit CPS**) i prebacujte **Latinica ↔ Ćirilica** bez grešaka u digrafima.
- **Detekcija promene govornika i pametno prelamanje**: Akustička detekcija automatski otvara nov titl kada progovori drugi sagovornik, dok lingvistička pravila prirodno seku duge rečenice na zapeti i balansirano prelamaju **1–4 reda** za TV/film ili Shorts/Reels.
- **Pametna analiza snimka i otporan EN/SR Auto-Detect**: Pozadinska analiza pri uvozu prepoznaje broj govornika i temu razgovora, dok `Auto (EN / SR)` sprečava da strani akcenti zalutaju u pogrešan jezik (uz ručni izbor za DE, ES, FR, IT, RU, EL i druge jezike).

---

## Izaberite režim rada po svojoj meri

| Režim | Privatnost | Brzina | Potreban hardver |
| :--- | :--- | :--- | :--- |
| **100% Oflajn (Lokalni AI)** | Nula mrežnog saobraćaja — sve ostaje na računaru | ~3x brže od snimka | NVIDIA GPU sa 4 GB+ VRAM (ili Vulkan / CPU) |
| **Hybrid Turbo (GPU + Cloud API)** | Audio ostaje lokalno; samo tekst titlova ide na DeepSeek / OpenAI | Do **28x brže od snimka** | Bilo koji GPU/CPU + DeepSeek ili OpenAI-kompatibilni API ključ |
| **Samo transkripcija (Bez LLM-a)** | 100% lokalno prepoznavanje govora i tajming | ~27–34x brže na GPU-u | Bilo koji moderan Windows 10/11 ili Linux računar |

---

## Počnite u 3 koraka

1. **Instalirajte**: Preuzmite **Windows Setup `.exe`** ili **Linux `.AppImage`** sa stranice **[GitHub Releases](https://github.com/Gruya13/EasySRT-Studio/releases/latest)**.
2. **Izaberite prevodioca**: Odaberite besplatan lokalni AI model prilagođen vašoj grafičkoj kartici ili povežite **DeepSeek / OpenAI-kompatibilni** API ključ.
3. **Ubacite snimak i izvezite**: Prevucite `.mp3`, `.mp4`, `.mkv`, `.webm`, `.wav`, `.m4a`, `.opus`, `.flac` ili postojeće `.srt`/`.vtt` titlove, doterajte na tajmlajnu i izvezite spremne **`.SRT`**, **`.VTT`** ili **`.TXT`** fajlove.

Trebaju vam prečice na tastaturi, saveti za grafičku karticu ili pomoć oko podešavanja? Pogledajte **[Korisnički vodič](docs/user/sr/guide.md)** i **[Rešavanje problema i FAQ](docs/user/sr/troubleshooting.md)**.

---

## Privatnost, bezbednost i ažuriranja

Vaši audio i video snimci nikada ne napuštaju vaš računar. API ključevi se čuvaju u bezbednom sistemskom trezoru (OS Keyring ili šifrovani lokalni trezor), potpisana ugrađena ažuriranja održavaju program aktuelnim jednim klikom, a predloge ili prijave grešaka bez ličnih podataka možete poslati direktno iz podnožja aplikacije.

---

## Licenca

Objavljeno pod licencom **[Apache License 2.0](LICENSE)** — Copyright © 2026 **IG-Systems**.

<p><sub>Inženjerska dokumentacija (EN): <a href="docs/dev/ARCHITECTURE.md">Arhitektura sistema</a> · <a href="docs/dev/BENCHMARKS.md">Empirijski benchmark rezultati</a> · <a href="docs/dev/CONTRIBUTING.md">Doprinos projektu</a></sub></p>
