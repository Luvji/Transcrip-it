use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineDescriptor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub capabilities: Vec<EngineCapability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineCapability {
    Offline,
    Cpu,
    Gpu,
    English,
    Multilingual,
    TimestampedSegments,
    SpeakerLabels,
    StructuredOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl EngineError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }

    pub fn cancelled() -> Self {
        Self::new("cancelled", "processing was cancelled", true)
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for EngineError {}

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), EngineError> {
        if self.is_cancelled() {
            Err(EngineError::cancelled())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineProgress {
    pub fraction: f32,
    pub stage: String,
}

pub trait ProgressReporter: Send + Sync {
    fn report(&self, progress: EngineProgress);
}

pub struct NoopProgress;

impl ProgressReporter for NoopProgress {
    fn report(&self, _progress: EngineProgress) {}
}

pub struct EngineContext<'a> {
    pub cancellation: &'a CancellationToken,
    pub progress: &'a dyn ProgressReporter,
}

#[derive(Clone, Debug)]
pub struct TranscriptionRequest {
    pub audio_path: PathBuf,
    pub language_hint: Option<String>,
    pub initial_prompt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegmentOutput {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub language_code: Option<String>,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionOutput {
    pub detected_language: Option<String>,
    pub segments: Vec<TranscriptSegmentOutput>,
}

pub trait TranscriptionEngine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;

    fn transcribe(
        &self,
        request: &TranscriptionRequest,
        context: &EngineContext<'_>,
    ) -> Result<TranscriptionOutput, EngineError>;
}

#[derive(Clone, Debug)]
pub struct SpeakerRequest {
    pub audio_path: PathBuf,
    pub segments: Vec<TranscriptSegmentOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAssignment {
    pub segment_index: usize,
    pub speaker_label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerOutput {
    pub assignments: Vec<SpeakerAssignment>,
}

pub trait SpeakerEngine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;

    fn assign_speakers(
        &self,
        request: &SpeakerRequest,
        context: &EngineContext<'_>,
    ) -> Result<SpeakerOutput, EngineError>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarySegmentInput {
    pub segment_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_label: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct SummaryRequest {
    pub meeting_id: String,
    pub template_id: String,
    pub output_language: String,
    pub segments: Vec<SummarySegmentInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryOutput {
    pub schema_version: u32,
    pub content: Value,
}

pub trait SummaryEngine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;

    fn summarize(
        &self,
        request: &SummaryRequest,
        context: &EngineContext<'_>,
    ) -> Result<SummaryOutput, EngineError>;
}

#[derive(Debug)]
pub enum RegistryError {
    EmptyEngineId,
    EngineNotFound { kind: &'static str, id: String },
    LockPoisoned,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyEngineId => write!(formatter, "engine id must not be empty"),
            Self::EngineNotFound { kind, id } => write!(formatter, "{kind} engine not found: {id}"),
            Self::LockPoisoned => write!(formatter, "engine registry lock is poisoned"),
        }
    }
}

impl Error for RegistryError {}

#[derive(Default)]
pub struct EngineRegistry {
    transcription: RwLock<HashMap<String, Arc<dyn TranscriptionEngine>>>,
    speakers: RwLock<HashMap<String, Arc<dyn SpeakerEngine>>>,
    summaries: RwLock<HashMap<String, Arc<dyn SummaryEngine>>>,
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_transcription(
        &self,
        engine: Arc<dyn TranscriptionEngine>,
    ) -> Result<Option<Arc<dyn TranscriptionEngine>>, RegistryError> {
        let id = validate_descriptor(engine.descriptor())?;
        self.transcription
            .write()
            .map_err(|_| RegistryError::LockPoisoned)
            .map(|mut engines| engines.insert(id, engine))
    }

    pub fn register_speaker(
        &self,
        engine: Arc<dyn SpeakerEngine>,
    ) -> Result<Option<Arc<dyn SpeakerEngine>>, RegistryError> {
        let id = validate_descriptor(engine.descriptor())?;
        self.speakers
            .write()
            .map_err(|_| RegistryError::LockPoisoned)
            .map(|mut engines| engines.insert(id, engine))
    }

    pub fn register_summary(
        &self,
        engine: Arc<dyn SummaryEngine>,
    ) -> Result<Option<Arc<dyn SummaryEngine>>, RegistryError> {
        let id = validate_descriptor(engine.descriptor())?;
        self.summaries
            .write()
            .map_err(|_| RegistryError::LockPoisoned)
            .map(|mut engines| engines.insert(id, engine))
    }

    pub fn transcription(&self, id: &str) -> Result<Arc<dyn TranscriptionEngine>, RegistryError> {
        self.transcription
            .read()
            .map_err(|_| RegistryError::LockPoisoned)?
            .get(id)
            .cloned()
            .ok_or_else(|| RegistryError::EngineNotFound {
                kind: "transcription",
                id: id.to_owned(),
            })
    }

    pub fn speaker(&self, id: &str) -> Result<Arc<dyn SpeakerEngine>, RegistryError> {
        self.speakers
            .read()
            .map_err(|_| RegistryError::LockPoisoned)?
            .get(id)
            .cloned()
            .ok_or_else(|| RegistryError::EngineNotFound {
                kind: "speaker",
                id: id.to_owned(),
            })
    }

    pub fn summary(&self, id: &str) -> Result<Arc<dyn SummaryEngine>, RegistryError> {
        self.summaries
            .read()
            .map_err(|_| RegistryError::LockPoisoned)?
            .get(id)
            .cloned()
            .ok_or_else(|| RegistryError::EngineNotFound {
                kind: "summary",
                id: id.to_owned(),
            })
    }
}

fn validate_descriptor(descriptor: EngineDescriptor) -> Result<String, RegistryError> {
    if descriptor.id.trim().is_empty() {
        Err(RegistryError::EmptyEngineId)
    } else {
        Ok(descriptor.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingProgress(Mutex<Vec<EngineProgress>>);

    impl ProgressReporter for RecordingProgress {
        fn report(&self, progress: EngineProgress) {
            self.0.lock().unwrap().push(progress);
        }
    }

    struct FakeTranscriber {
        version: &'static str,
    }

    impl TranscriptionEngine for FakeTranscriber {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                id: "fake-transcriber".to_owned(),
                name: "Fake transcriber".to_owned(),
                version: self.version.to_owned(),
                capabilities: vec![
                    EngineCapability::Offline,
                    EngineCapability::TimestampedSegments,
                ],
            }
        }

        fn transcribe(
            &self,
            _request: &TranscriptionRequest,
            context: &EngineContext<'_>,
        ) -> Result<TranscriptionOutput, EngineError> {
            context.cancellation.check()?;
            context.progress.report(EngineProgress {
                fraction: 1.0,
                stage: "complete".to_owned(),
            });
            Ok(TranscriptionOutput {
                detected_language: Some("en".to_owned()),
                segments: vec![TranscriptSegmentOutput {
                    start_ms: 0,
                    end_ms: 1000,
                    text: self.version.to_owned(),
                    language_code: Some("en".to_owned()),
                    confidence: Some(0.9),
                }],
            })
        }
    }

    fn request() -> TranscriptionRequest {
        TranscriptionRequest {
            audio_path: PathBuf::from("fixture.wav"),
            language_hint: Some("en".to_owned()),
            initial_prompt: None,
        }
    }

    #[test]
    fn registry_resolves_and_replaces_engines_by_stable_id() {
        let registry = EngineRegistry::new();
        registry
            .register_transcription(Arc::new(FakeTranscriber { version: "v1" }))
            .unwrap();
        let replaced = registry
            .register_transcription(Arc::new(FakeTranscriber { version: "v2" }))
            .unwrap();
        assert!(replaced.is_some());

        let engine = registry.transcription("fake-transcriber").unwrap();
        let progress = RecordingProgress(Mutex::new(Vec::new()));
        let cancellation = CancellationToken::new();
        let output = engine
            .transcribe(
                &request(),
                &EngineContext {
                    cancellation: &cancellation,
                    progress: &progress,
                },
            )
            .unwrap();

        assert_eq!(engine.descriptor().version, "v2");
        assert_eq!(output.segments[0].text, "v2");
        assert_eq!(progress.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn cancellation_is_shared_and_stops_work_before_execution() {
        let engine = FakeTranscriber { version: "v1" };
        let cancellation = CancellationToken::new();
        let cloned = cancellation.clone();
        cloned.cancel();

        let result = engine.transcribe(
            &request(),
            &EngineContext {
                cancellation: &cancellation,
                progress: &NoopProgress,
            },
        );

        assert_eq!(result.unwrap_err().code, "cancelled");
    }

    #[test]
    fn missing_engines_return_a_typed_error() {
        let error = match EngineRegistry::new().transcription("missing") {
            Ok(_) => panic!("missing engine should fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            RegistryError::EngineNotFound {
                kind: "transcription",
                ..
            }
        ));
    }
}
