export interface EventRow {
  id: number;
  ts: number;
  kind: string;
  app: string | null;
  title: string | null;
  duration_ms: number | null;
  mode: string | null;
  ocr_text: string | null;
  vision_desc?: string;
}

export interface VaultItem {
  id: number;
  ts: number;
  idea: string;
  items: string;
  favorited: boolean;
}

export interface SessionGroup {
  title: string;
  distraction: boolean;
  events: EventRow[];
  total_duration_ms: number;
}

export interface SummariesResponse {
  generated_at: number;
  groups: SessionGroup[];
}

export interface FocusWindow {
  hour_start: number;
  hour_end: number;
  total_focus_ms: number;
  label: string;
}

export interface AppCluster {
  apps: string[];
  session_count: number;
}

export interface DriftOrigin {
  app: string;
  from_app: string;
  count: number;
}

export interface HeatmapData {
  cells: number[];
  max_value: number;
  day_labels: string[];
  hour_labels: string[];
}

export interface RhythmReport {
  focus_windows: FocusWindow[];
  fingerprint: AppCluster[];
  drift_origins: DriftOrigin[];
  heatmap: HeatmapData;
}
