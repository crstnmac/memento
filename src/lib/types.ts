export interface MemoryRow {
  id: number;
  source: string;
  app: string | null;
  title: string;
  content: string;
  createdAt: number;
  isMeeting: boolean;
  openLoop: boolean;
  enrichment?: MemoryEnrichment;
}

export interface StructuredMemory {
  summary: string;
  people: string[];
  topics: string[];
  decisions: string[];
  commitments: string[];
  dates: string[];
  entities: string[];
  actionItems: Array<{
    content: string;
    urgent: boolean;
    remindAt: number | null;
  }>;
}

export interface MemoryEnrichment {
  memoryId: number;
  provider: string;
  model: string;
  structured: StructuredMemory;
  enrichedAt: number;
}

export interface MemoryVersion {
  id: number;
  threadId: number;
  parentVersionId: number | null;
  createdAt: number;
  isFrozen: boolean;
  wordCount: number;
  content: string;
  itemCount: number;
}

export interface AppVisit {
  id: number;
  app: string;
  windowTitle: string | null;
  startedAt: number;
  endedAt: number | null;
}

export interface ThreadRow {
  id: number;
  source: string;
  app: string | null;
  title: string;
  derivative: string | null;
  createdAt: number;
  itemCount: number;
  openCount: number;
  lastActivity: number;
}

export interface ActionItem {
  id: number;
  memoryId: number | null;
  content: string;
  status: string;
  createdAt: number;
  completedAt: number | null;
  isUrgent: boolean;
  isUnread: boolean;
  sortOrder: number;
  remindAt: number | null;
  remindDate: string | null;
  remindSlot: string | null;
  reminderShown: boolean;
}

export interface ReminderBatch {
  id: string;
  label: string;
  items: ActionItem[];
}

export interface DueReminders {
  dnd: boolean;
  batches: ReminderBatch[];
  total: number;
}

export interface McpState {
  running: boolean;
  port: number;
  url: string;
  bridgePath: string;
}

export type FollowUpCategory = "chat" | "email" | "meeting" | "productivity";

/** An app or service Memento can read and find follow-ups from. */
export interface FollowUpSource {
  id: string;
  name: string;
  category: FollowUpCategory;
  /** Installed-app bundle ids whose icon represents the source (may be empty). */
  bundleIds: string[];
}

export interface FollowUpSources {
  sources: FollowUpSource[];
  /** Source ids the user switched off. */
  disabled: string[];
}

export interface CaptureAppSelection {
  name: string;
  bundleId: string;
}

export interface Conversation {
  id: number;
  title: string;
  transcript: string | null;
  audioPath: string | null;
  startedAt: number;
  endedAt: number | null;
  durationMs: number | null;
  kind: "meeting" | "voice-note";
}

export interface ActivityConversation {
  id: number;
  app: string;
  title: string;
  startedAt: number;
  updatedAt: number;
  beforeChars: number;
  newChars: number;
  messageCount: number;
}

export interface ConversationEvent {
  id: number;
  conversationId: number;
  memoryId: number | null;
  capturedAt: number;
  beforeChars: number;
  newChars: number;
  newContent: string;
}

export interface Status {
  trusted: boolean;
  paused: boolean;
  recording: boolean;
  memoryCount: number;
  actionCount: number;
  dbPath: string;
  /** When a timed pause ends (epoch ms); null for no pause or an open-ended one. */
  pauseUntil?: number | null;
}

export type PermState = "granted" | "denied" | "notDetermined" | "restricted";

export interface Permissions {
  accessibility: PermState;
  microphone: PermState;
  screenRecording: PermState;
}

export interface Settings {
  transcriptionModel: string;
  recordMic: boolean;
  recordSystem: boolean;
  mixAudio: boolean;
  captureIntervalMs: number;
  onboardingComplete: boolean;
  autoStartMeetings: boolean;
  autoStopMeetings: boolean;
  maxCaptureDurationSec: number;
  reminderNudges: boolean;
  mcpEnabled: boolean;
  mcpPort: number;
  llmEnabled: boolean;
  llmProvider: string;
  llmBaseUrl: string;
  llmModel: string;
  llmHasApiKey: boolean;
  vaultPath: string;
  recordingKind: "meeting" | "voice-note";
}

export interface VaultStatus {
  path: string;
  exists: boolean;
  generatedFiles: number;
  importedNotes: number;
  lastSyncedAt: number | null;
}

export interface VaultSyncReport {
  exported: number;
  imported: number;
  updated: number;
  unchanged: number;
  path: string;
}

export interface RecordingStart {
  conversationId: number;
  startedMs: number;
}

export interface RecordingResult {
  conversationId: number;
  micPath: string | null;
  systemPath: string | null;
  durationMs: number;
  kind: "meeting" | "voice-note";
}

export interface RecordingAutoStopped {
  result: RecordingResult;
  reason: string;
}

export type RecordingMode = "mic" | "system" | "both";

export interface TranscriptionStarted {
  conversationId: number;
}

export interface TranscriptionFinished {
  conversationId: number;
  memoryId: number;
}

export interface TranscriptionFailed {
  conversationId: number;
  error: string;
}

export interface ModelDownloadProgress {
  model: string;
  received: number;
  total: number | null;
}

export interface TranscriptionModelStatus {
  ready: boolean;
  bytes: number;
}

export type TranscriptionModel = "whisper-tiny.en" | "whisper-tiny";

export const TRANSCRIPTION_MODELS: Array<{
  id: TranscriptionModel;
  label: string;
}> = [
  { id: "whisper-tiny.en", label: "English-only" },
  { id: "whisper-tiny", label: "Multilingual" },
];

export const DEFAULT_TRANSCRIPTION_MODEL: TranscriptionModel = "whisper-tiny.en";
