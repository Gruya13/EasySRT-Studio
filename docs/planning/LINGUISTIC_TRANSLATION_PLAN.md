# EasySRT Studio — Serbian Linguistic & Translation Quality Plan

**Status:** Planned (post-`v0.4.0` linguistic engine roadmap)  
**Target Releases:** `v0.4.x` / `v0.5.0`  
**Origin:** Comparative linguistic analysis of human production Serbian broadcast subtitles against the current LLM prompt and post-processing architecture.

This document defines six focused engineering and linguistic workstreams designed to bring EasySRT Studio's Serbian subtitle output in line with top-tier broadcast subtitling practice while actually surpassing hurried human production translations on English calques and punctuation hygiene.

**Core Linguistic Insight:** Real-world Serbian broadcast and interview subtitles are neither aggressively informal (*"ti"* across every speaker and context) nor stiffly bureaucratic/literary (*"smatrate"*, *"neophodno je"*, bookish infinitives). Instead, high-quality Serbian subtitles combine **context-appropriate address (`Vi` vs. `ti`)**, **living contemporary Ekavian syntax (`da` + present)**, **authentic domain lexicon**, **zero English semantic or structural calques**, and **clean Serbian orthography**.

---

## Summary of Planned Workstreams

| # | Workstream | Target Module(s) |
| :-: | :--- | :--- |
| **1** | **Decoupled Address Register (`Auto` / `Vi` / `ti`) & Natural Broadcast Style** | [`src-tauri/src/models.rs`](../../src-tauri/src/models.rs), [`src/types/models.ts`](../../src/types/models.ts), [`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs), [`src-tauri/src/pipeline/probe.rs`](../../src-tauri/src/pipeline/probe.rs), [`src/components/SettingsPanel.tsx`](../../src/components/SettingsPanel.tsx) |
| **2** | **Prompt De-Overfitting & Domain-Agnostic Linguistic Rules** | [`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs) |
| **3** | **Anti-Calque, Semantic Naturalization & Pleonasm Guardrails** | [`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs) |
| **4** | **Serbian Broadcast Punctuation & Discourse-Marker Hygiene** | [`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs), [`src-tauri/src/pipeline/sr_script.rs`](../../src-tauri/src/pipeline/sr_script.rs) |
| **5** | **Domain-Aware Lexical & Register Adaptation via Intake Probe (`topic_context`)** | [`src-tauri/src/pipeline/probe.rs`](../../src-tauri/src/pipeline/probe.rs), [`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs) |
| **6** | **Multi-Genre Human-Reference Evaluation & Regression Rubric** | `docs/`, offline local evaluation fixtures (`test/`, gitignored) |

---

## Workstream 1 — Decoupled Address Register (`Auto` / `Vi` / `ti`) & Natural Broadcast Style

### Current Behavior & Gap
1. In [`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs) (`build_serbian_style_guidelines`), address formality (`Vi` vs. `ti`) is tightly coupled to `TranslationStyle` (`Natural` vs. `Formal`, passed as `formal_style: bool` in `PromptOptions`):
   - When `formal_style == false` (`Natural`), the prompt aggressively forces informal `ti` (*„rekao si“*, *„šta misliš“*, *„važi“*), instructing the model to avoid `Vi` almost everywhere. In Serbian TV, press-conference, and sports interviews, however, reporters address interviewees with polite `Vi` (*„Rekli ste…“*, *„Šta mislite…“*, *„Kako biste opisali…“*) while speaking completely natural, conversational Serbian (`da` + present: *„morate da vodite računa“*, *„možemo da naučimo“*).
   - When `formal_style == true` (`Formal`), polite `Vi` is bundled with stiff journalistic/literary phrasing (*„smatrate“*, *„oceniti“*, *„neophodno je“*, bookish infinitives) that sounds overly rigid for spoken broadcast interviews.
2. Generic English *"you"* frequently drifts between `Vi` and `ti` inside the same answer or across consecutive cues when no explicit intra-turn consistency rule is enforced.

### Planned Changes
1. **Orthogonal `AddressForm` Contract (`models.rs` & `models.ts`)**:
   - Introduce `AddressForm` (`Auto` | `PoliteVi` | `InformalTi`, default `Auto`) on `JobOptions` in [`src-tauri/src/models.rs`](../../src-tauri/src/models.rs) (`#[serde(default)]`) and `addressForm?: AddressForm` in [`src/types/models.ts`](../../src/types/models.ts), preserving 100% backward compatibility with existing checkpoints and project files.
   - Thread `address_form: AddressForm` into `PromptOptions` in [`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs), and expose an optional selector in [`src/components/SettingsPanel.tsx`](../../src/components/SettingsPanel.tsx) (`Auto` by default so casual users need zero extra clicks).
2. **Recalibrate `Natural` vs. `Formal` Stylistic Tone (`translator.rs`)**:
   - **`Natural` (*Prirodni govorni*)**: Governs **syntactic and lexical naturalness** — living contemporary Ekavian Serbian, `da` + present constructions (*„moramo da naučimo“*, *„treba da pazimo“*), and natural spoken cadence — while choosing `Vi` vs. `ti` dynamically from `address_form` (in `Auto`: polite `Vi` for press conferences, TV/radio interviews, and public figures; informal `ti` for casual podcasts, vlogs, friends, and peer dialogue).
   - **`Formal` (*Standardni / formalni*)**: Governs **standard broadcast/documentary register** — polished standard Ekavian Serbian without slang or colloquialisms, pairing polite `Vi` (or impersonal/passive-reflexive broadcast phrasing) with natural clarity instead of bureaucratic phrasing.
3. **Intra-Turn Consistency for Generic *"you"***:
   - Instruct the LLM to maintain one consistent rendering of generic English *"you"* across a speaker's turn (`context_before`, `translate_cues`, `context_after`) — either consistent polite `Vi` (*„morate da vodite računa… treba Vam…“*), consistent generic `ti`, or impersonal `čovek` / reflexive `se`, never mixing `ti` and `Vi` toward the same addressee.

### Planned Unit Tests / Verification
- `translator::tests`: test the full `(Natural | Formal) × (Auto | PoliteVi | InformalTi)` prompt matrix across `BatchOperation::Translate` and `BatchOperation::PolishSerbian`, verifying that `Natural + PoliteVi` and `Natural + Auto` permit natural spoken Serbian with polite `Vi` without forcing stiff literary vocabulary.
- `models::tests`: verify serde backward-compatibility when loading legacy `v0.3.x` / `v0.4.0` projects lacking `addressForm` (defaults cleanly to `AddressForm::Auto`).

---

## Workstream 2 — Prompt De-Overfitting & Domain-Agnostic Linguistic Rules

### Current Behavior & Gap
- Uncommitted working-tree examples in `build_serbian_style_guidelines` ([`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs)) currently include sentences lifted almost verbatim from a specific local basketball interview clip.
- Baking lines from a single test file into production system prompts creates prompt overfitting: it artificially inflates quality on that single clip while offering brittle guidance on podcasts, documentaries, tech talks, films, or news reports, and consumes system-prompt tokens on every batch across all languages/topics.

### Planned Changes
1. **Replace Clip-Specific Lines with General Linguistic Principles (`translator.rs`)**:
   - Remove test-clip-derived sentences from `build_serbian_style_guidelines`.
   - Replace them with concise, domain-neutral contrast pairs spanning three representative contexts (a press/TV interview, everyday peer dialogue, and documentary narration) so local 4B models (`gemma-3-4b`) and cloud models (`deepseek-chat`) learn the underlying structural distinctions rather than memorizing one scenario.
2. **Fixture Isolation Engineering Policy**:
   - Codify an explicit project rule: phrases from evaluation or human-reference test clips must **never** be copied verbatim into production prompts, dictionaries, or deterministic post-processors (except genuinely universal Serbian lexical/orthographic patterns).

### Planned Unit Tests / Verification
- `translator::tests`: add prompt-hygiene assertions verifying clean differentiation between `Natural` and `Formal` guidelines using domain-neutral examples, with zero test-fixture strings embedded in production prompts.

---

## Workstream 3 — Anti-Calque, Semantic Naturalization & Pleonasm Guardrails

### Current Behavior & Gap
- Comparison against real-world human broadcast subtitles shows that even experienced human translators frequently introduce English lexical and syntactic calques under deadline pressure, and unguided LLMs mirror those same habits:
  - **Verb & phrase calques:** translating leadership/coaching *"challenged us"* literally as *„izazvao nas je“* (which carries a confrontational/duel connotation in Serbian instead of *„pozvao nas je na odgovornost“* / *„tražio je od nas“*), or *"I feel like..."* literally as *„osećam da…“* instead of *„mislim da…“* / *„imam utisak da…“* / *„čini mi se da…“*.
  - **Quantifier & idiom calques:** literal *"a ton of energy"* (*„tona energije“*) where natural Serbian prefers *„mnogo energije“* / *„ogromna energija“*, or redundant *"use the crowd to our advantage"* (*„iskoristiti publiku u svoju korist“*) instead of *„iskoristiti podršku/energiju publike“*.
  - **Stacked English adverbs & pleonasms:** *"probably just"* rendered literally as *„verovatno jednostavno“* instead of *„verovatno prosto“* / *„prosto“*, and *"still"* rendered as *„još uvek“* where concise *„još“* is cleaner and saves subtitle CPS.
  - **Enclitic stranding across cue boundaries:** Serbian enclitics (`je`, `sam`, `ste`, `nam`, `vam`, `se`) placed after pauses or stranded in weak syntactic positions.

### Planned Changes
1. **Targeted Anti-Calque Rules in `build_serbian_style_guidelines` (`translator.rs`)**:
   - **Pragmatic Meaning Over Dictionary Equivalents:** Instruct the model to translate communicative intent rather than first-match dictionary verbs (*I feel like* → *mislim da / čini mi se*; non-combative *challenge someone to do better* → *pozvati na odgovornost / tražiti od nekoga*, not *izazvati*).
   - **Anti-Pleonasm & Conciseness:** Explicitly discourage stacked English modifier pairs (*verovatno jednostavno* → *prosto / verovatno samo*), repetitive verb-noun pairings (*iskoristiti X u svoju korist* → *iskoristiti podršku/prednost X*), and verbose *još uvek* when *još* preserves identical meaning at lower CPS.
   - **Idiomatic Authenticity:** Preserve established Serbian domain idioms (*upirati prstom*, *pretrpeti poraz*, *idemo dalje*) rather than flattening them into word-for-word English structures.
   - **Enclitic & Word-Order Reinforcement:** Keep Wackernagel second-position enclitic placement and active/reflexive voice rules concise and prominent for both `Translate` and `PolishSerbian`.

### Planned Unit Tests / Verification
- `translator::tests`: assert that anti-calque, anti-pleonasm, and enclitic-placement directives appear in `Translate` (`target_language == "sr"`) and `PolishSerbian` prompts without exceeding compact prompt-token budgets for local 4B models.

---

## Workstream 4 — Serbian Broadcast Punctuation & Discourse-Marker Hygiene

### Current Behavior & Gap
1. **English Conjunction-Comma Calque:** English ASR transcripts frequently place a comma after sentence-initial conjunctions (`"And, I feel..."`, `"So, it's a tough loss..."`, `"But, you know..."`). Both LLMs and human translators often copy this verbatim into Serbian as *„I, osećam…“*, *„Tako da, težak je poraz…“*, or *„Ali, to je…“*. Under Serbian orthography (*Pravopis*), a comma after `I`, `A`, `Ali`, `Pa`, or `Tako da` is incorrect unless it opens a genuine parenthetical insertion bounded by a closing comma (e.g., *„I, naravno, pobedili smo.“*).
2. **Discourse-Marker Balance (`remove_fillers`):** In live speech, repeated conversational fillers (*znate*, *nekako*, *i slično*) reflect spoken rhythm in verbatim mode (`remove_fillers = false`), whereas clean broadcast subtitles (`remove_fillers = true`) should strip redundant discourse padding cleanly to keep reading speed (CPS) comfortable without altering speaker meaning.

### Planned Changes
1. **Prompt Orthography Rule (`translator.rs`)**:
   - Add an explicit Serbian punctuation guideline prohibiting English-style commas immediately after sentence-initial conjunctions (`I,`, `A,`, `Ali,`, `Pa,`, `Tako da,`) unless delimiting a true parenthetical clause/phrase.
2. **Conservative Deterministic Conjunction-Comma Cleanup (`translator.rs` or `sr_script.rs`)**:
   - Implement a targeted post-processing pass for Serbian output that strips unambiguous stray commas directly after sentence- or clause-initial conjunctions (`^|([.!?…]\s+)(I|A|Ali|Pa|Tako da),\s+`) **only when no matching closing parenthetical comma follows within the clause**, preserving valid parentheticals such as *„I, naravno, uspeli smo.“* or *„Ali, iskreno, nije bilo lako.“*.
3. **Clear `remove_fillers` Behavioral Separation (`translator.rs`)**:
   - Keep `remove_fillers = true` focused on removing semantically empty spoken padding (*znate*, *nekako*, *i slično*, *you know*, *kind of*, *and stuff*) while strictly preserving semantic uses (*„Da li znate?“*, *„Ne znam kako“*).
   - Keep `remove_fillers = false` strictly faithful to verbatim discourse markers while still cleaning invalid English-only punctuation calques.

### Planned Unit Tests / Verification
- Unit tests covering deterministic comma cleanup across Latin and Cyrillic inputs:
  - Strips invalid English calques: `"I, mislim da možemo."` → `"I mislim da možemo."`; `"Tako da, težak je poraz."` → `"Tako da težak je poraz."`.
  - Preserves valid Serbian parentheticals: `"I, naravno, idemo dalje."`, `"Ali, iskreno govoreći, nije bilo lako."`, `"Pa, znate, sezona je duga."` (when `znate` is retained).

---

## Workstream 5 — Domain-Aware Lexical & Register Adaptation via Intake Probe

### Current Behavior & Gap
- `classify_media_topic` in [`src-tauri/src/pipeline/translator.rs`](../../src-tauri/src/pipeline/translator.rs) (called by [`src-tauri/src/pipeline/probe.rs`](../../src-tauri/src/pipeline/probe.rs)) currently emits `topic`, `suggestedStyle` (`"natural"` | `"formal"`), and `estimatedSpeakers` (`1..=3`).
- In `build_system_prompt_with_options`, `AUDIO TOPIC & DOMAIN CONTEXT` instructs the LLM only to disambiguate technical, specialized, or proper-noun terminology; it does not inform Serbian register selection (`Vi` vs. `ti`) or domain-authentic Serbian phrasing (e.g., sports journalism: *partija*, *gostovanja*, *pretrpeti poraz*, *svlačionica*, *konferencija za medije* vs. casual gaming vlog vs. medical lecture).

### Planned Changes
1. **Extend `MediaTopicClassification` with Optional Address Suggestion (`translator.rs` & `probe.rs`)**:
   - Add `#[serde(default, alias = "suggestedAddress", ...)] suggested_address: Option<String>` to `TopicClassificationResponse` and `pub suggested_address: Option<AddressForm>` to `MediaTopicClassification`.
   - Update `classify_media_topic` rules so interviews, press conferences, news anchors, and formal panels suggest `"vi"` (paired with `natural` for sports/TV interviews or `formal` for news/academic lectures), casual monologues/podcasts/vlogs suggest `"ti"`, and ambiguous cases return `"auto"`.
2. **Domain-Terminology Guidance in `AUDIO TOPIC & DOMAIN CONTEXT` (`translator.rs`)**:
   - Expand the existing two-line domain block concisely to instruct the translator to use authentic Serbian terminology, idioms, and conversational register native to that domain (`clean_topic`) — without bloating token count.
3. **Resilient Fallback (`probe.rs` & `appStore`)**:
   - When `address_form` in `JobOptions` is `AddressForm::Auto`, apply the probe's inferred `suggested_address` (if present) as a contextual hint while letting explicit user selection (`PoliteVi` or `InformalTi`) override unconditionally.

### Planned Unit Tests / Verification
- `translator::tests`: verify `TopicClassificationResponse` deserializes cleanly both with and without `suggestedAddress` (backward compatibility with older local models that only return 2–3 keys).
- `translator::tests`: verify `build_system_prompt_with_options` surfaces `topic_context` and domain-register instructions cleanly when `topic_context` is set.

---

## Workstream 6 — Multi-Genre Human-Reference Evaluation & Regression Rubric

### Current Behavior & Gap
- Empirical benchmarking in [`docs/dev/BENCHMARKS.md`](../dev/BENCHMARKS.md) tracks runtime latency, cue counts, CPS warnings, and qualitative spot-checks on 10 sample clips, but there is no standardized Serbian linguistic rubric comparing prompt iterations and local vs. cloud models against real human broadcast subtitles across multiple genres.

### Planned Changes
1. **Local-Only Multi-Genre Evaluation Corpus (Gitignored)**:
   - Store reference human translations and test clips strictly outside tracked source/prompt files (e.g., under gitignored `test/`) across at least four distinct genres:
     1. **Broadcast / Sports Press Interview** (Q&A with polite `Vi` + living spoken syntax).
     2. **Casual Podcast / Vlog Dialogue** (peer `ti` register + high filler density).
     3. **Documentary / News Broadcast** (standard literary/journalistic Ekavian + impersonal/formal constructions).
     4. **Dramatic / Narrative Monologue** (expressive cadence, tight enclitic rhythm, and poetic/emotional tone).
2. **6-Axis Serbian Linguistic Quality Rubric**:
   Evaluate every prompt or post-processing change on both local (`gemma-3-4b-it-Q4_K_M` or newer) and cloud (`deepseek-chat`) models across six criteria (scored `1–5` per clip):
   1. **Register & Address Consistency:** Appropriate `Vi` vs. `ti` choice and zero mid-turn switching on generic *"you"*.
   2. **Ekavian Purity & Morphology:** Zero Ijekavian/Croatian lexical leakage; accurate Serbian case endings and gender agreement.
   3. **Anti-Calque & Idiomatic Naturalness:** Absence of literal English verb/phrase calques (*izazvao nas*, *osećam da*, stacked adverbs) and natural Serbian `da` + present cadence where appropriate.
   4. **Enclitic Placement & Word Order:** Natural Wackernagel second-slot enclitics without stranded boundary clitics.
   5. **Serbian Orthography & Punctuation:** Clean punctuation with zero English `I,` / `Tako da,` conjunction-comma artifacts.
   6. **Subtitle Readability & CPS Compliance:** Clean line wrapping, no 1-word orphan starters, `>= 80 ms` breathing gaps, and comfortable reading speed.

---

## Suggested Implementation Order (`v0.4.x` → `v0.5.0`)

1. **Phase 1 — Prompt Cleanup & Linguistic Guardrails (Workstreams 2, 3 & 4 — Prompt + Safe Cleanup)**:
   - Replace clip-specific examples in `build_serbian_style_guidelines` with domain-neutral contrast pairs.
   - Add anti-calque, anti-pleonasm, intra-turn generic *"you"* consistency, and Serbian punctuation rules, plus the conservative deterministic conjunction-comma cleaner and unit tests.
2. **Phase 2 — Decoupled Address Register (`Auto` / `Vi` / `ti`) (Workstream 1)**:
   - Add `AddressForm` to `JobOptions` (`models.rs`, `models.ts`), `PromptOptions` (`translator.rs`), and `SettingsPanel.tsx`, decoupling polite/informal address from `Natural` vs. `Formal` stylistic tone.
3. **Phase 3 — Intake Probe Register & Domain Integration (Workstream 5)**:
   - Extend `classify_media_topic` (`MediaTopicClassification`) with optional `suggested_address` inference and enhance `AUDIO TOPIC & DOMAIN CONTEXT` domain-terminology guidance.
4. **Phase 4 — Multi-Genre Benchmark Gate (Workstream 6)**:
   - Run the 6-axis evaluation rubric across all four genres on both local GGUF and DeepSeek endpoints before locking prompt defaults for release.

### Mandatory Verification Gate Before Merging Changes
Per `AGENTS.md`, any code changes implementing these workstreams must pass the full local verification suite on `staging`:
```bash
npm run version:check
npm run typecheck
npm test
npm run build
cd src-tauri && cargo check --all-targets && cargo test && cargo clippy --all-targets -- -D warnings
```
