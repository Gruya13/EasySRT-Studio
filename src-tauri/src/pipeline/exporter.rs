//! Subtitle formatters and file exporter (`.srt`, `.vtt`, `.txt`, optional UTF-8 BOM,
//! and dual source + translated `.srt` output).

use crate::models::{ExportFormat, ExportOptions, ExportResult, SubtitleCue, SubtitleProject};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Formats milliseconds as an SRT timestamp (`HH:MM:SS,mmm`).
pub fn format_srt_timestamp(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

/// Formats milliseconds as a WebVTT timestamp (`HH:MM:SS.mmm`).
pub fn format_vtt_timestamp(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn select_cue_text(cue: &SubtitleCue, use_source: bool) -> &str {
    if use_source || cue.translated_text.trim().is_empty() {
        cue.source_text.trim()
    } else {
        cue.translated_text.trim()
    }
}

/// Generates an `.srt` document string.
pub fn format_srt(cues: &[SubtitleCue], use_source: bool, utf8_bom: bool) -> String {
    let mut out = String::new();
    if utf8_bom {
        out.push('\u{FEFF}');
    }

    for (idx, cue) in cues.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let seq = idx + 1;
        let start = format_srt_timestamp(cue.start_ms);
        let end = format_srt_timestamp(cue.end_ms.max(cue.start_ms));
        let text = select_cue_text(cue, use_source);
        out.push_str(&format!("{seq}\n{start} --> {end}\n{text}\n"));
    }

    out
}

/// Generates a `.vtt` (WebVTT) document string.
pub fn format_vtt(cues: &[SubtitleCue], use_source: bool, utf8_bom: bool) -> String {
    let mut out = String::new();
    if utf8_bom {
        out.push('\u{FEFF}');
    }
    out.push_str("WEBVTT\n\n");

    for (idx, cue) in cues.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let seq = idx + 1;
        let start = format_vtt_timestamp(cue.start_ms);
        let end = format_vtt_timestamp(cue.end_ms.max(cue.start_ms));
        let text = select_cue_text(cue, use_source);
        out.push_str(&format!("{seq}\n{start} --> {end}\n{text}\n"));
    }

    out
}

/// Generates a clean plain-text `.txt` transcript without timestamps.
pub fn format_txt(cues: &[SubtitleCue], use_source: bool, utf8_bom: bool) -> String {
    let mut out = String::new();
    if utf8_bom {
        out.push('\u{FEFF}');
    }

    for (idx, cue) in cues.iter().enumerate() {
        let raw = select_cue_text(cue, use_source);
        let single_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if single_line.is_empty() {
            continue;
        }
        if idx > 0 && !out.ends_with('\n') && out != "\u{FEFF}" {
            out.push('\n');
        }
        out.push_str(&single_line);
        out.push('\n');
    }

    out
}

fn resolve_export_dir(project: &SubtitleProject, options: &ExportOptions) -> Result<PathBuf> {
    if !options.save_beside_audio {
        if let Some(dir) = options.output_dir.as_deref() {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                return Ok(PathBuf::from(trimmed));
            }
        }
    }

    if let Some(parent) = Path::new(&project.audio_path).parent() {
        if !parent.as_os_str().is_empty() {
            return Ok(parent.to_path_buf());
        }
    }

    if let Some(dir) = options.output_dir.as_deref() {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    std::env::current_dir().context("Failed to determine export output directory")
}

fn resolve_stem(project: &SubtitleProject) -> String {
    Path::new(&project.audio_file_name)
        .file_stem()
        .or_else(|| Path::new(&project.audio_path).file_stem())
        .and_then(|s| s.to_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("subtitles")
        .to_string()
}

/// Exports `project` subtitles according to `options` and returns the written file paths.
pub fn export_project_files(
    project: &SubtitleProject,
    options: &ExportOptions,
) -> Result<ExportResult> {
    let out_dir = resolve_export_dir(project, options)?;
    fs::create_dir_all(&out_dir).with_context(|| {
        format!(
            "Failed to create export directory: {}",
            out_dir.to_string_lossy()
        )
    })?;

    let stem = resolve_stem(project);
    let has_translation = project
        .cues
        .iter()
        .any(|c| !c.translated_text.trim().is_empty());

    let src_lang = project
        .detected_language
        .as_deref()
        .filter(|s| !s.is_empty() && *s != "auto")
        .unwrap_or_else(|| {
            let s = project.options.source_language.trim();
            if !s.is_empty() && s != "auto" {
                s
            } else {
                "src"
            }
        });

    let target_lang = if has_translation {
        let t = project.options.target_language.trim();
        if !t.is_empty() {
            t
        } else {
            "sr"
        }
    } else {
        src_lang
    };

    let formats = if options.formats.is_empty() {
        vec![ExportFormat::Srt]
    } else {
        options.formats.clone()
    };

    let mut written_files = Vec::new();

    for fmt in &formats {
        let (ext, content) = match fmt {
            ExportFormat::Srt => ("srt", format_srt(&project.cues, false, options.utf8_bom)),
            ExportFormat::Vtt => ("vtt", format_vtt(&project.cues, false, options.utf8_bom)),
            ExportFormat::Txt => ("txt", format_txt(&project.cues, false, options.utf8_bom)),
        };

        let file_name = format!("{stem}.{target_lang}.{ext}");
        let file_path = out_dir.join(file_name);
        fs::write(&file_path, content.as_bytes()).with_context(|| {
            format!(
                "Failed to write exported subtitle file: {}",
                file_path.to_string_lossy()
            )
        })?;
        written_files.push(file_path.to_string_lossy().to_string());
    }

    if options.include_source_srt {
        let src_tag = if src_lang == target_lang {
            format!("{src_lang}.source")
        } else {
            src_lang.to_string()
        };
        let src_file_path = out_dir.join(format!("{stem}.{src_tag}.srt"));
        let src_content = format_srt(&project.cues, true, options.utf8_bom);
        fs::write(&src_file_path, src_content.as_bytes()).with_context(|| {
            format!(
                "Failed to write source SRT file: {}",
                src_file_path.to_string_lossy()
            )
        })?;
        written_files.push(src_file_path.to_string_lossy().to_string());
    }

    Ok(ExportResult { written_files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{JobOptions, JobPhase};

    fn sample_cues() -> Vec<SubtitleCue> {
        vec![
            SubtitleCue {
                id: 1,
                start_ms: 500,
                end_ms: 2750,
                source_text: "Hello everyone!".to_string(),
                translated_text: "Здраво свима!\nЧеститамо Ђорђу.".to_string(),
                cps: 12.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 2,
                start_ms: 3000,
                end_ms: 5125,
                source_text: "Testing Serbian characters.".to_string(),
                translated_text: "Тест слова: čćžšđ ČĆŽŠĐ ЉЊЏ.".to_string(),
                cps: 13.0,
                warnings: vec![],
            },
        ]
    }

    #[test]
    fn formats_srt_with_and_without_bom_preserving_diacritics() {
        let cues = sample_cues();
        let srt = format_srt(&cues, false, false);
        assert!(!srt.starts_with('\u{FEFF}'));
        assert!(srt.contains("1\n00:00:00,500 --> 00:00:02,750\nЗдраво свима!\nЧеститамо Ђорђу.\n"));
        assert!(srt.contains("2\n00:00:03,000 --> 00:00:05,125\nТест слова: čćžšđ ČĆŽŠĐ ЉЊЏ.\n"));

        let srt_bom = format_srt(&cues, false, true);
        assert!(srt_bom.starts_with('\u{FEFF}'));
        assert_eq!(&srt_bom['\u{FEFF}'.len_utf8()..], srt);
    }

    #[test]
    fn formats_vtt_and_txt() {
        let cues = sample_cues();
        let vtt = format_vtt(&cues, false, false);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:00.500 --> 00:00:02.750"));

        let txt = format_txt(&cues, false, false);
        assert_eq!(
            txt,
            "Здраво свима! Честитамо Ђорђу.\nТест слова: čćžšđ ČĆŽŠĐ ЉЊЏ.\n"
        );
    }

    #[test]
    fn exports_files_including_source_srt() {
        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_export_test_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let project = SubtitleProject {
            job_id: "job-1".to_string(),
            audio_path: temp_dir.join("epizoda01.mp3").to_string_lossy().to_string(),
            audio_file_name: "epizoda01.mp3".to_string(),
            duration_ms: 6000,
            detected_language: Some("en".to_string()),
            options: JobOptions::default(),
            words: vec![],
            cues: sample_cues(),
            phase: JobPhase::Completed,
            completed_translation_batches: 1,
            error_message: None,
            updated_at: "2026-11-22T18:00:00Z".to_string(),
        };

        let opts = ExportOptions {
            output_dir: None,
            save_beside_audio: true,
            include_source_srt: true,
            formats: vec![ExportFormat::Srt, ExportFormat::Vtt, ExportFormat::Txt],
            utf8_bom: true,
        };

        let res = export_project_files(&project, &opts).unwrap();
        assert_eq!(res.written_files.len(), 4);

        let sr_srt = fs::read_to_string(temp_dir.join("epizoda01.sr.srt")).unwrap();
        assert!(sr_srt.starts_with('\u{FEFF}'));
        assert!(sr_srt.contains("čćžšđ ČĆŽŠĐ"));

        let en_srt = fs::read_to_string(temp_dir.join("epizoda01.en.srt")).unwrap();
        assert!(en_srt.contains("Hello everyone!"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
