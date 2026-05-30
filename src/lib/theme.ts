export type ThemeName = 'paper' | 'charcoal' | 'forest' | 'slate';

export interface ThemeMeta {
  name: ThemeName;
  label: string;
  bg: string;
  accent: string;
}

export const THEMES: ThemeMeta[] = [
  { name: 'paper', label: 'Paper', bg: '#FEFCF4', accent: '#F16A01' },
  { name: 'charcoal', label: 'Charcoal', bg: '#161412', accent: '#FF7A1A' },
  { name: 'forest', label: 'Forest', bg: '#0F1712', accent: '#E8A41E' },
  { name: 'slate', label: 'Slate', bg: '#F7F8FA', accent: '#3B5BDB' },
];

const STORAGE_KEY = 'ccube-theme';

function isThemeName(value: string | null): value is ThemeName {
  return value !== null && THEMES.some((t) => t.name === value);
}

export function getInitialTheme(): ThemeName {
  if (typeof window === 'undefined') return 'paper';

  const stored = window.localStorage.getItem(STORAGE_KEY);
  if (isThemeName(stored)) return stored;

  const prefersDark = window.matchMedia?.('(prefers-color-scheme: dark)').matches;
  return prefersDark ? 'charcoal' : 'paper';
}

export function applyTheme(name: ThemeName): void {
  if (typeof document === 'undefined') return;

  // Paper is the :root default — drop the attribute so :root is the single source of truth.
  if (name === 'paper') {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = name;
  }

  if (typeof window !== 'undefined') {
    window.localStorage.setItem(STORAGE_KEY, name);
  }
}

export function nextTheme(current: ThemeName): ThemeName {
  const idx = THEMES.findIndex((t) => t.name === current);
  const next = THEMES[(idx + 1) % THEMES.length];
  return next.name;
}
