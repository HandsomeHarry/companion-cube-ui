You are consolidating Companion Cube's pattern memory. It has grown organically
over weeks and now contains overlapping, redundant, or over-specific entries.
Rewrite it as a tighter, more abstract version that covers the same cases with
fewer, clearer rules.

## The person

{profile}

## Current patterns

{patterns}

## Retained corrections from the last 30 days

(Ground truth. Your rewrite must continue to honor these. If your rewrite would
cause any of these to be re-triggered as drift, it will fail evaluation.)

{retained_corrections}

## Your rewrite

Produce a new patterns.md. Guidelines:

1. Fewer entries, more abstract. Aim for 6-10 total.
2. Each entry covers a general principle with examples. The detector will apply
   them to cases it hasn't seen.
3. Preserve mode-specific distinctions when behavior genuinely differs by mode.
   Collapse them when it doesn't.
4. Stay under 1500 chars total — leave room for future growth.
5. Keep the § separator. One principle per line.

Explain your consolidation decisions in the rationale field — which entries you
merged, which you abstracted, which you removed as stale.

Respond in JSON: {schema}
