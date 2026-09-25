use crate::database::{Database, TranscriptSegmentRecord};
use serde::Serialize;
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceReference {
    segment_id: String,
    start_ms: i64,
    end_ms: i64,
    source_track: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundedNote {
    text: String,
    evidence: Vec<EvidenceReference>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionNote {
    text: String,
    owner: String,
    due_date: String,
    evidence: Vec<EvidenceReference>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingNotes {
    meeting_id: String,
    strategy: String,
    overview: Vec<GroundedNote>,
    topics: Vec<GroundedNote>,
    decisions: Vec<GroundedNote>,
    actions: Vec<ActionNote>,
    questions: Vec<GroundedNote>,
    risks: Vec<GroundedNote>,
    next_steps: Vec<GroundedNote>,
}

impl EvidenceReference {
    fn from_segment(segment: &TranscriptSegmentRecord) -> Self {
        Self {
            segment_id: segment.id.clone(),
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            source_track: segment.source_track.clone(),
        }
    }
}

fn grounded(segment: &TranscriptSegmentRecord) -> GroundedNote {
    GroundedNote {
        text: segment.display_text.trim().to_owned(),
        evidence: vec![EvidenceReference::from_segment(segment)],
    }
}

fn contains_marker(text: &str, markers: &[&str]) -> bool {
    let normalized = text.to_lowercase();
    markers.iter().any(|marker| normalized.contains(marker))
}

fn select_segments(
    segments: &[TranscriptSegmentRecord],
    markers: &[&str],
    limit: usize,
) -> Vec<GroundedNote> {
    segments
        .iter()
        .filter(|segment| contains_marker(&segment.display_text, markers))
        .take(limit)
        .map(grounded)
        .collect()
}

fn generate_notes(
    meeting_id: &str,
    segments: &[TranscriptSegmentRecord],
) -> Result<MeetingNotes, String> {
    if segments.is_empty() {
        return Err("This meeting does not have a transcript yet.".to_owned());
    }
    let overview = segments.iter().take(3).map(grounded).collect::<Vec<_>>();
    let mut topic_candidates = segments
        .iter()
        .filter(|segment| segment.display_text.split_whitespace().count() >= 5)
        .collect::<Vec<_>>();
    topic_candidates
        .sort_by_key(|segment| std::cmp::Reverse(segment.display_text.split_whitespace().count()));
    topic_candidates.truncate(5);
    topic_candidates.sort_by_key(|segment| segment.start_ms);
    let topics = topic_candidates.into_iter().map(grounded).collect();
    let action_segments = segments
        .iter()
        .filter(|segment| {
            contains_marker(
                &segment.display_text,
                &[
                    "action item",
                    "todo",
                    "to-do",
                    "we need to",
                    "i will",
                    "we will",
                    "should follow",
                ],
            )
        })
        .take(20)
        .collect::<Vec<_>>();
    let actions = action_segments
        .into_iter()
        .map(|segment| ActionNote {
            text: segment.display_text.trim().to_owned(),
            owner: "Not specified".to_owned(),
            due_date: "Not specified".to_owned(),
            evidence: vec![EvidenceReference::from_segment(segment)],
        })
        .collect();
    let notes = MeetingNotes {
        meeting_id: meeting_id.to_owned(),
        strategy: "Local extractive draft; review required".to_owned(),
        overview,
        topics,
        decisions: select_segments(
            segments,
            &[
                "decided",
                "decision",
                "agreed",
                "approved",
                "we will use",
                "we'll use",
            ],
            20,
        ),
        actions,
        questions: segments
            .iter()
            .filter(|segment| segment.display_text.trim_end().ends_with('?'))
            .take(20)
            .map(grounded)
            .collect(),
        risks: select_segments(
            segments,
            &["risk", "blocker", "blocked", "concern", "problem", "delay"],
            20,
        ),
        next_steps: select_segments(
            segments,
            &[
                "next step",
                "follow up",
                "follow-up",
                "going forward",
                "we need to",
            ],
            20,
        ),
    };
    validate_notes(&notes)?;
    Ok(notes)
}

fn validate_grounded(note: &GroundedNote) -> bool {
    !note.text.trim().is_empty()
        && !note.evidence.is_empty()
        && note.evidence.iter().all(|evidence| {
            !evidence.segment_id.is_empty()
                && evidence.start_ms >= 0
                && evidence.end_ms >= evidence.start_ms
        })
}

fn validate_notes(notes: &MeetingNotes) -> Result<(), String> {
    let grounded = notes
        .overview
        .iter()
        .chain(&notes.topics)
        .chain(&notes.decisions)
        .chain(&notes.questions)
        .chain(&notes.risks)
        .chain(&notes.next_steps);
    if grounded.into_iter().any(|note| !validate_grounded(note))
        || notes.actions.iter().any(|action| {
            action.text.trim().is_empty()
                || action.owner.trim().is_empty()
                || action.due_date.trim().is_empty()
                || action.evidence.is_empty()
        })
    {
        return Err("Generated notes failed evidence validation.".to_owned());
    }
    Ok(())
}

fn timestamp(milliseconds: i64) -> String {
    format!(
        "{:02}:{:02}",
        milliseconds / 60_000,
        (milliseconds / 1000) % 60
    )
}

fn evidence_label(evidence: &[EvidenceReference]) -> String {
    evidence
        .iter()
        .map(|item| format!("[{}]", timestamp(item.start_ms)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn push_section(content: &mut String, heading: &str, notes: &[GroundedNote], markdown: bool) {
    if markdown {
        content.push_str(&format!("## {heading}\n\n"));
    } else {
        content.push_str(&format!("{heading}\n{}\n", "-".repeat(heading.len())));
    }
    if notes.is_empty() {
        content.push_str("Not identified in the transcript.\n\n");
        return;
    }
    for note in notes {
        content.push_str(&format!(
            "- {} {}\n",
            note.text,
            evidence_label(&note.evidence)
        ));
    }
    content.push('\n');
}

fn format_notes(title: &str, notes: &MeetingNotes, markdown: bool) -> String {
    let mut content = if markdown {
        format!("# {title} — Meeting notes\n\n_{}_\n\n", notes.strategy)
    } else {
        format!(
            "{title} — Meeting notes\n{}\n\n{}\n\n",
            "=".repeat(title.len() + 16),
            notes.strategy
        )
    };
    push_section(&mut content, "Overview", &notes.overview, markdown);
    push_section(&mut content, "Topics", &notes.topics, markdown);
    push_section(&mut content, "Decisions", &notes.decisions, markdown);
    if markdown {
        content.push_str("## Actions\n\n");
    } else {
        content.push_str("Actions\n-------\n");
    }
    if notes.actions.is_empty() {
        content.push_str("Not identified in the transcript.\n\n");
    } else {
        for action in &notes.actions {
            content.push_str(&format!(
                "- {} — Owner: {}; Due: {} {}\n",
                action.text,
                action.owner,
                action.due_date,
                evidence_label(&action.evidence)
            ));
        }
        content.push('\n');
    }
    push_section(&mut content, "Questions", &notes.questions, markdown);
    push_section(&mut content, "Risks", &notes.risks, markdown);
    push_section(&mut content, "Next steps", &notes.next_steps, markdown);
    content
}

#[tauri::command]
pub fn generate_meeting_notes(
    database: tauri::State<'_, Database>,
    meeting_id: String,
) -> Result<MeetingNotes, String> {
    let segments = database
        .list_transcript(&meeting_id)
        .map_err(|error| error.to_string())?;
    generate_notes(&meeting_id, &segments)
}

#[tauri::command]
pub fn export_meeting_notes(
    database: tauri::State<'_, Database>,
    meeting_id: String,
    format: String,
    review_confirmed: bool,
) -> Result<String, String> {
    if !review_confirmed {
        return Err("Review the meeting notes and confirm them before exporting.".to_owned());
    }
    if !matches!(format.as_str(), "markdown" | "text") {
        return Err("Export format must be markdown or text.".to_owned());
    }
    let title = database
        .meeting_title(&meeting_id)
        .map_err(|error| error.to_string())?;
    let segments = database
        .list_transcript(&meeting_id)
        .map_err(|error| error.to_string())?;
    let notes = generate_notes(&meeting_id, &segments)?;
    let recording_path = database
        .meeting_recording_path(&meeting_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "This meeting has no recording directory.".to_owned())?;
    let export_directory = PathBuf::from(recording_path).join("exports");
    fs::create_dir_all(&export_directory)
        .map_err(|error| format!("Could not create export directory: {error}"))?;
    let extension = if format == "markdown" { "md" } else { "txt" };
    let path = export_directory.join(format!("notes.{extension}"));
    fs::write(&path, format_notes(&title, &notes, format == "markdown"))
        .map_err(|error| format!("Could not write notes export: {error}"))?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(id: &str, start_ms: i64, text: &str) -> TranscriptSegmentRecord {
        TranscriptSegmentRecord {
            id: id.to_owned(),
            sequence_number: start_ms / 1000,
            start_ms,
            end_ms: start_ms + 900,
            speaker_label: Some("Speaker".to_owned()),
            source_track: Some("mic".to_owned()),
            source_text: text.to_owned(),
            display_text: text.to_owned(),
            active_correction_id: None,
        }
    }

    #[test]
    fn notes_are_extractive_grounded_and_preserve_unknowns() {
        let segments = vec![
            segment(
                "s1",
                0,
                "We discussed the release plan and current recording quality.",
            ),
            segment(
                "s2",
                1_000,
                "We decided to test the Ubuntu package tomorrow.",
            ),
            segment(
                "s3",
                2_000,
                "Action item: we need to verify the clean installation.",
            ),
            segment("s4", 3_000, "Is the microphone warning visible?"),
            segment("s5", 4_000, "The main risk is a delayed model download."),
        ];
        let notes = generate_notes("meeting-1", &segments).unwrap();
        assert_eq!(notes.decisions[0].evidence[0].segment_id, "s2");
        assert_eq!(notes.actions[0].owner, "Not specified");
        assert_eq!(notes.actions[0].due_date, "Not specified");
        assert_eq!(notes.questions[0].evidence[0].start_ms, 3_000);
        assert_eq!(notes.risks[0].text, segments[4].display_text);
        assert!(format_notes("Test", &notes, true).contains("[00:02]"));
    }

    #[test]
    fn notes_require_a_transcript() {
        assert!(generate_notes("meeting-1", &[]).is_err());
    }
}
