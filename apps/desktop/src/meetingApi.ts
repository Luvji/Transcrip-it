import { invoke, isTauri } from "@tauri-apps/api/core";

export type MeetingState =
  | "draft"
  | "recording"
  | "paused"
  | "processing"
  | "ready"
  | "failed"
  | "archived";

export interface Meeting {
  id: string;
  title: string;
  state: MeetingState;
  sourceKind: string;
  durationMs: number | null;
  recordingPath: string | null;
  createdAt: string;
  updatedAt: string;
  tags: string[];
  transcriptSegmentCount: number;
}

interface CreateMeetingInput {
  id: string;
  idempotencyKey: string;
  title: string;
}

const browserStorageKey = "transcrip-it.browser-meetings";

function browserMeetings(): Meeting[] {
  const stored = localStorage.getItem(browserStorageKey);
  return stored ? (JSON.parse(stored) as Meeting[]) : [];
}

function saveBrowserMeetings(meetings: Meeting[]) {
  localStorage.setItem(browserStorageKey, JSON.stringify(meetings));
}

function browserUpdate(id: string, update: (meeting: Meeting) => Meeting) {
  const meetings = browserMeetings();
  const index = meetings.findIndex((meeting) => meeting.id === id);
  if (index < 0) throw new Error(`Meeting not found: ${id}`);
  meetings[index] = update(meetings[index]);
  saveBrowserMeetings(meetings);
}

export const meetingApi = {
  async list(includeArchived = false): Promise<Meeting[]> {
    if (isTauri()) return invoke("list_meetings", { includeArchived });
    return browserMeetings()
      .filter((meeting) => includeArchived || meeting.state !== "archived")
      .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt));
  },

  async create(input: CreateMeetingInput): Promise<Meeting> {
    if (isTauri()) return invoke("create_meeting", { input });
    const existing = browserMeetings().find((meeting) => meeting.id === input.id);
    if (existing) return existing;
    const timestamp = new Date().toISOString();
    const meeting: Meeting = {
      id: input.id,
      title: input.title.trim(),
      state: "draft",
      sourceKind: "recording",
      durationMs: null,
      recordingPath: null,
      createdAt: timestamp,
      updatedAt: timestamp,
      tags: [],
      transcriptSegmentCount: 0,
    };
    saveBrowserMeetings([meeting, ...browserMeetings()]);
    return meeting;
  },

  async rename(meetingId: string, title: string): Promise<void> {
    if (isTauri()) return invoke("rename_meeting", { meetingId, title });
    browserUpdate(meetingId, (meeting) => ({ ...meeting, title: title.trim(), updatedAt: new Date().toISOString() }));
  },

  async replaceTags(meetingId: string, tags: string[]): Promise<string[]> {
    if (isTauri()) return invoke("replace_meeting_tags", { meetingId, tags });
    const normalized = [...new Map(tags.map((tag) => [tag.trim().toLowerCase(), tag.trim()])).values()]
      .filter(Boolean)
      .sort((left, right) => left.localeCompare(right));
    browserUpdate(meetingId, (meeting) => ({ ...meeting, tags: normalized, updatedAt: new Date().toISOString() }));
    return normalized;
  },

  async archive(meetingId: string): Promise<void> {
    const idempotencyKey = crypto.randomUUID();
    if (isTauri()) return invoke("archive_meeting", { meetingId, idempotencyKey });
    browserUpdate(meetingId, (meeting) => ({ ...meeting, state: "archived", updatedAt: new Date().toISOString() }));
  },

  async reopen(meetingId: string): Promise<void> {
    const idempotencyKey = crypto.randomUUID();
    if (isTauri()) return invoke("reopen_meeting", { meetingId, idempotencyKey });
    browserUpdate(meetingId, (meeting) => ({ ...meeting, state: "draft", updatedAt: new Date().toISOString() }));
  },

  async delete(meetingId: string): Promise<void> {
    if (isTauri()) {
      await invoke("delete_meeting", { meetingId });
      return;
    }
    saveBrowserMeetings(browserMeetings().filter((meeting) => meeting.id !== meetingId));
  },
};

export interface RecordingStatus {
  active: boolean;
  meetingId: string | null;
  elapsedSeconds: number;
  processRunning: boolean;
  paused: boolean;
  levels: { name: string; peak: number }[];
  storageAvailableBytes: number | null;
}

export interface CompletedRecording {
  meetingId: string;
  durationMs: number;
  recordingPath: string;
}

export interface StartRecordingOptions {
  mode: "mic" | "system" | "both";
  micSource: string | null;
  systemSource: string | null;
  echoCancellation: boolean;
}

export interface AudioDevices {
  defaultSource: string;
  defaultSink: string;
  systemSource: string | null;
  sources: { name: string; monitor: boolean }[];
}

export const recordingApi = {
  available: isTauri(),
  async start(meetingId: string, consentConfirmed: boolean, options: StartRecordingOptions): Promise<RecordingStatus> {
    if (!isTauri()) throw new Error("Audio capture is available in the Transcrip-it desktop app.");
    return invoke("start_recording", { meetingId, consentConfirmed, options });
  },
  async devices(): Promise<AudioDevices> {
    if (!isTauri()) return { defaultSource: "", defaultSink: "", systemSource: null, sources: [] };
    return invoke("audio_devices");
  },
  async status(): Promise<RecordingStatus> {
    if (!isTauri()) return { active: false, meetingId: null, elapsedSeconds: 0, processRunning: false, paused: false, levels: [], storageAvailableBytes: null };
    return invoke("recording_status");
  },
  async stop(meetingId: string): Promise<CompletedRecording> {
    if (!isTauri()) throw new Error("Audio capture is available in the Transcrip-it desktop app.");
    return invoke("stop_recording", { meetingId });
  },
  async pause(meetingId: string): Promise<RecordingStatus> {
    if (!isTauri()) throw new Error("Audio capture is available in the Transcrip-it desktop app.");
    return invoke("pause_recording", { meetingId });
  },
  async resume(meetingId: string): Promise<RecordingStatus> {
    if (!isTauri()) throw new Error("Audio capture is available in the Transcrip-it desktop app.");
    return invoke("resume_recording", { meetingId });
  },
  async play(meetingId: string, track: "mic" | "system", startMs = 0): Promise<void> {
    if (!isTauri()) throw new Error("Audio playback is available in the Transcrip-it desktop app.");
    await invoke("play_recording_track", { meetingId, track, startMs });
  },
};

export interface TranscriptSegment {
  id: string;
  sequenceNumber: number;
  startMs: number;
  endMs: number;
  speakerLabel: string | null;
  sourceText: string;
}

export interface TranscriptionModelStatus {
  installed: boolean;
  pack: string;
  modelBytes: number | null;
  engineVersion: string;
}

export interface TranscriptSearchResult {
  meetingId: string;
  meetingTitle: string;
  segmentId: string;
  startMs: number;
  snippet: string;
}

export const transcriptionApi = {
  available: isTauri(),
  async modelStatus(): Promise<TranscriptionModelStatus> {
    if (!isTauri()) return { installed: false, pack: "Balanced English", modelBytes: null, engineVersion: "Desktop app required" };
    return invoke("transcription_model_status");
  },
  async installModel(): Promise<TranscriptionModelStatus> {
    if (!isTauri()) throw new Error("Model installation is available in the Transcrip-it desktop app.");
    return invoke("install_transcription_model");
  },
  async transcribe(meetingId: string): Promise<TranscriptSegment[]> {
    if (!isTauri()) throw new Error("Local transcription is available in the Transcrip-it desktop app.");
    return invoke("transcribe_meeting", { meetingId });
  },
  async list(meetingId: string): Promise<TranscriptSegment[]> {
    if (!isTauri()) return [];
    return invoke("list_transcript", { meetingId });
  },
  async search(query: string): Promise<TranscriptSearchResult[]> {
    if (!isTauri() || query.trim().length < 2) return [];
    return invoke("search_transcripts", { query });
  },
  async export(meetingId: string, format: "markdown" | "text"): Promise<string> {
    if (!isTauri()) throw new Error("Transcript export is available in the Transcrip-it desktop app.");
    return invoke("export_transcript", { meetingId, format });
  },
};
