# Model context settings: owner-aware precedence B

Both `codex-app-server --model-variants-capability` and `codex debug models --capabilities`
report `modelVariants:true`, `modelContextSettingsV1:true` and `modelContextSettingsV2:true`.
The V2 capability identifies B; V1 alone identifies the earlier A implementation.
Both probes bypass configuration and credential loading.

Root `model_context_settings_v1` remains the enable bit (default false). Root window,
absolute compaction and `model_auto_compact_percent` remain Profile defaults. Marker false
keeps pre-r290 Profile-first flat-field behavior and ignores the new layered metadata.

`ModelInfo.context_settings` optionally contains `catalog`, `owner` and `preserved` objects.
Each has optional `context_window`, `auto_compact_percent` and `auto_compact_token_limit`.
Missing/null values inherit. Percent is 1..100; percent and absolute cannot coexist at one
level. Compaction is an atomic category, chosen independently from the window.

With the marker true:

- Window: preserved > owner > Profile > catalog > flat `max_context_window` / unknown.
  The flat maximum caps the result.
- Compaction: preserved > owner > Profile > catalog > default90%. Percentages use the
  resolved nominal window with integer flooring and a widened intermediate. The effective
  threshold remains capped at90% of that window, before usable-window headroom.
- A flat-only row supplies catalog defaults. Native retains these original defaults in
  `context_settings.catalog` before replacing flat runtime values, so serialization/cache
  round trips and later Profile changes retain their provenance. When layered metadata is
  present, flat values are compatibility data rather than a second owner layer.
- Native consumes preservation metadata but never creates or edits owner/preserved values.
  Cutex authors migration. Removing a preserved field/category immediately exposes its
  owner's original value; removing owner values exposes the Profile and catalog defaults.

For a synthesized model absent from the catalog, an explicit Profile window wins. Without
one, reported capacity stays unknown while internal compaction and Guardian budgets use
272000 as a last resort. Default compaction is244800; percent100 is capped244800 and
Profile absolute500000 without a window is also capped244800. The fallback does not cap
an explicit Profile window. It is runtime-only state, excluded from JSON/schema/TS, and is
not inferred for every catalog row with a null window. Context exhaustion marks usage full
enough to trigger later compaction in Total and BodyAfterPrefix scopes while keeping
unknown reported capacity unknown.

The resolution snapshot follows live model switches and child/reviewer configuration.
Guardian request/image/scorer budgets use the same resolved model. Reviewer cache identity
includes the marker and Profile percent. Existing longest-prefix/single-namespace matching
and exact-only local configurations carrying `request_model` remain unchanged.

Layered metadata is preserved through catalog/cache serialization and local resolution.
It is model-selection metadata, never an inference-provider request parameter. No provider
request DTO gains a field. TUI usage displays continue to use actual capacity, rather than
substituting an unresolved Profile or internal budget fallback.

Migration must preserve effective numbers before enabling B. The caller keeps legacy mode
when equality is unrepresentable, including malformed windows above a known maximum or
legacy prefix-scoped raw thresholds above90%. Existing running agents keep their startup
snapshots and runtime variant. Native does not migrate persistent Profiles.
