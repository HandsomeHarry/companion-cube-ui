export interface EventRow {
  id: number;
  ts: number;
  kind: string;
  app: string | null;
  title: string | null;
  duration_ms: number | null;
  mode: string | null;
  ocr_text: string | null;
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
