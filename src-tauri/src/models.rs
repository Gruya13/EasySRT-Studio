//! Canonical 1:1 shared contract with `src/types/models.ts`.
//! All structs and enums use `#[serde(rename_all = "camelCase")]`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const EVENT_DOWNLOAD_PROGRESS: &str = "download-progress";
pub const EVENT_JOB_PROGRESS: &str = "job-progress";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageMode {
    Installed,
    Portable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoragePaths {
    pub mode: StorageMode,
    pub root_dir: String,
    pub bin_dir: String,
    pub models_dir: String,
    pub data_dir: String,
    pub config_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VramTier {
    CpuOnly,
    Vram4Gb,
    Vram8Gb,
    Vram12GbPlus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub has_nvidia: bool,
    pub gpu_name: Option<String>,
    pub vram_mb: u64,
    pub recommended_tier: VramTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogItemKind {
    Ffmpeg,
    CudaRuntime,
    WhisperBin,
    LlamaBin,
    VadModel,
    WhisperModel,
    LlmModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveType {
    Zip,
    TarGz,
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogItem {
    pub id: String,
    pub name: String,
    pub kind: CatalogItemKind,
    pub platform: String,
    pub version: String,
    pub url: String,
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub vram_tier: Option<VramTier>,
    pub target_subdir: String,
    pub executable_name: Option<String>,
    pub archive_type: ArchiveType,
    pub description: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogManifest {
    pub schema_version: u32,
    pub updated_at: String,
    pub items: Vec<CatalogItem>,
}

impl Default for CatalogManifest {
    fn default() -> Self {
        Self {
            schema_version: 1,
            updated_at: String::new(),
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledComponentStatus {
    pub id: String,
    pub installed: bool,
    pub path: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub gpu: GpuInfo,
    pub storage: StoragePaths,
    pub components: Vec<InstalledComponentStatus>,
    pub ready_for_offline: bool,
    pub missing_required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgressEvent {
    pub item_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
    pub stage: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SegmentPreset {
    #[default]
    Standard,
    Reels,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentRules {
    pub preset: SegmentPreset,
    pub max_chars_per_line: u32,
    pub max_lines: u32,
    pub min_duration_ms: u64,
    pub max_duration_ms: u64,
    pub max_cps: f64,
}

impl Default for SegmentRules {
    fn default() -> Self {
        Self {
            preset: SegmentPreset::Standard,
            max_chars_per_line: 42,
            max_lines: 2,
            min_duration_ms: 1000,
            max_duration_ms: 7000,
            max_cps: 17.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SerbianScript {
    #[default]
    Latin,
    Cyrillic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TranslationProviderKind {
    #[default]
    LocalLlama,
    DeepSeek,
    CustomOpenAi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomModelEntry {
    pub id: String,
    pub name: String,
    pub kind: CatalogItemKind,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationConfig {
    pub provider: TranslationProviderKind,
    pub deepseek_model: String,
    pub custom_base_url: String,
    pub custom_model: String,
    pub batch_size: u32,
    pub context_window: u32,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            provider: TranslationProviderKind::LocalLlama,
            deepseek_model: "deepseek-chat".to_string(),
            custom_base_url: String::new(),
            custom_model: String::new(),
            batch_size: 25,
            context_window: 2048,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Srt,
    Vtt,
    Txt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub output_dir: Option<String>,
    pub save_beside_audio: bool,
    pub include_source_srt: bool,
    pub formats: Vec<ExportFormat>,
    pub utf8_bom: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            output_dir: None,
            save_beside_audio: true,
            include_source_srt: false,
            formats: vec![ExportFormat::Srt],
            utf8_bom: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub ui_language: String,
    pub models_dir_override: Option<String>,
    pub active_whisper_model_id: Option<String>,
    pub active_llm_model_id: Option<String>,
    pub custom_models: Vec<CustomModelEntry>,
    pub translation: TranslationConfig,
    pub default_segment_rules: SegmentRules,
    pub default_script: SerbianScript,
    pub export: ExportOptions,
    pub has_completed_setup: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui_language: "en".to_string(),
            models_dir_override: None,
            active_whisper_model_id: None,
            active_llm_model_id: None,
            custom_models: Vec::new(),
            translation: TranslationConfig::default(),
            default_segment_rules: SegmentRules::default(),
            default_script: SerbianScript::Latin,
            export: ExportOptions::default(),
            has_completed_setup: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedWord {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default)]
    pub probability: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleCue {
    pub id: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub source_text: String,
    pub translated_text: String,
    pub cps: f64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobOptions {
    pub source_language: String,
    pub target_language: String,
    pub script: SerbianScript,
    pub segment_rules: SegmentRules,
    pub glossary: BTreeMap<String, String>,
    pub polish_serbian: bool,
    pub provider_override: Option<TranslationProviderKind>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            source_language: "auto".to_string(),
            target_language: "sr".to_string(),
            script: SerbianScript::Latin,
            segment_rules: SegmentRules::default(),
            glossary: BTreeMap::new(),
            polish_serbian: true,
            provider_override: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobPhase {
    Queued,
    ConvertingAudio,
    Transcribing,
    Segmenting,
    Translating,
    Polishing,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleProject {
    pub job_id: String,
    pub audio_path: String,
    pub audio_file_name: String,
    pub duration_ms: u64,
    pub detected_language: Option<String>,
    pub options: JobOptions,
    pub words: Vec<TimedWord>,
    pub cues: Vec<SubtitleCue>,
    pub phase: JobPhase,
    pub completed_translation_batches: u32,
    pub error_message: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgressEvent {
    pub job_id: String,
    pub phase: JobPhase,
    pub percent: f64,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub written_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub has_deep_seek_key: bool,
    pub has_custom_api_key: bool,
    pub storage_backend: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enum_camel_case_serialization_matches_ts_contract() {
        assert_eq!(serde_json::to_value(StorageMode::Installed).unwrap(), json!("installed"));
        assert_eq!(serde_json::to_value(StorageMode::Portable).unwrap(), json!("portable"));

        assert_eq!(serde_json::to_value(VramTier::CpuOnly).unwrap(), json!("cpuOnly"));
        assert_eq!(serde_json::to_value(VramTier::Vram4Gb).unwrap(), json!("vram4Gb"));
        assert_eq!(serde_json::to_value(VramTier::Vram8Gb).unwrap(), json!("vram8Gb"));
        assert_eq!(serde_json::to_value(VramTier::Vram12GbPlus).unwrap(), json!("vram12GbPlus"));

        assert_eq!(serde_json::to_value(CatalogItemKind::Ffmpeg).unwrap(), json!("ffmpeg"));
        assert_eq!(serde_json::to_value(CatalogItemKind::CudaRuntime).unwrap(), json!("cudaRuntime"));
        assert_eq!(serde_json::to_value(CatalogItemKind::WhisperBin).unwrap(), json!("whisperBin"));
        assert_eq!(serde_json::to_value(CatalogItemKind::LlamaBin).unwrap(), json!("llamaBin"));
        assert_eq!(serde_json::to_value(CatalogItemKind::VadModel).unwrap(), json!("vadModel"));
        assert_eq!(serde_json::to_value(CatalogItemKind::WhisperModel).unwrap(), json!("whisperModel"));
        assert_eq!(serde_json::to_value(CatalogItemKind::LlmModel).unwrap(), json!("llmModel"));

        assert_eq!(serde_json::to_value(ArchiveType::Zip).unwrap(), json!("zip"));
        assert_eq!(serde_json::to_value(ArchiveType::TarGz).unwrap(), json!("tarGz"));
        assert_eq!(serde_json::to_value(ArchiveType::Raw).unwrap(), json!("raw"));

        assert_eq!(serde_json::to_value(SegmentPreset::Standard).unwrap(), json!("standard"));
        assert_eq!(serde_json::to_value(SegmentPreset::Reels).unwrap(), json!("reels"));
        assert_eq!(serde_json::to_value(SegmentPreset::Custom).unwrap(), json!("custom"));

        assert_eq!(serde_json::to_value(SerbianScript::Latin).unwrap(), json!("latin"));
        assert_eq!(serde_json::to_value(SerbianScript::Cyrillic).unwrap(), json!("cyrillic"));

        assert_eq!(serde_json::to_value(TranslationProviderKind::LocalLlama).unwrap(), json!("localLlama"));
        assert_eq!(serde_json::to_value(TranslationProviderKind::DeepSeek).unwrap(), json!("deepSeek"));
        assert_eq!(serde_json::to_value(TranslationProviderKind::CustomOpenAi).unwrap(), json!("customOpenAi"));

        assert_eq!(serde_json::to_value(ExportFormat::Srt).unwrap(), json!("srt"));
        assert_eq!(serde_json::to_value(ExportFormat::Vtt).unwrap(), json!("vtt"));
        assert_eq!(serde_json::to_value(ExportFormat::Txt).unwrap(), json!("txt"));

        assert_eq!(serde_json::to_value(JobPhase::Queued).unwrap(), json!("queued"));
        assert_eq!(serde_json::to_value(JobPhase::ConvertingAudio).unwrap(), json!("convertingAudio"));
        assert_eq!(serde_json::to_value(JobPhase::Transcribing).unwrap(), json!("transcribing"));
        assert_eq!(serde_json::to_value(JobPhase::Segmenting).unwrap(), json!("segmenting"));
        assert_eq!(serde_json::to_value(JobPhase::Translating).unwrap(), json!("translating"));
        assert_eq!(serde_json::to_value(JobPhase::Polishing).unwrap(), json!("polishing"));
        assert_eq!(serde_json::to_value(JobPhase::Completed).unwrap(), json!("completed"));
        assert_eq!(serde_json::to_value(JobPhase::Cancelled).unwrap(), json!("cancelled"));
        assert_eq!(serde_json::to_value(JobPhase::Failed).unwrap(), json!("failed"));
    }

    #[test]
    fn key_field_names_match_ts_contract() {
        let cfg = serde_json::to_value(TranslationConfig::default()).unwrap();
        assert!(cfg.get("deepseekModel").is_some());
        assert!(cfg.get("customBaseUrl").is_some());
        assert!(cfg.get("customModel").is_some());
        assert!(cfg.get("batchSize").is_some());
        assert!(cfg.get("contextWindow").is_some());

        let keys = serde_json::to_value(ApiKeyStatus {
            has_deep_seek_key: true,
            has_custom_api_key: false,
            storage_backend: "keyring".to_string(),
        })
        .unwrap();
        assert!(keys.get("hasDeepSeekKey").is_some());
        assert!(keys.get("hasCustomApiKey").is_some());
        assert!(keys.get("storageBackend").is_some());
    }
}
