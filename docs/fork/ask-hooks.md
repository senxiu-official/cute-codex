# Fork: ExternalInput owner admission and gate control (cutex ask hooks)

Fork-only API for the trusted launch owner (cutex). It implements N1 and N2 of
`cutex-desktop/docs/cutex-agent-asks.md` §12.2. The model never sees these
controls. Every method checks the private launch binding (owner, thread,
runtime generation) like the other `thread/externalInput/*` methods, and all
of them are `experimental` (the client must opt into the experimental API).

## Capability

`initialize` returns `externalInputControlVersion: 1` when the server runs with
an ExternalInput launch binding. Older servers omit the field. Version 1 means:

- `admission: "exclusive_turn"` on `thread/externalInput/submit`;
- `thread/externalInput/resume`;
- `thread/externalInput/interrupt`.

Older servers reject `admission` as an unknown field (`deny_unknown_fields`),
so a client that sends it without checking the capability fails loudly.

## N1: exclusive admission

`thread/externalInput/submit` takes an optional `admission`:

- absent or `"queue"`: today's behaviour;
- `"exclusive_turn"`: start a dedicated turn now, or refuse and queue nothing.

The field is not part of the envelope and does not change the semantic digest.
It requires an active delivery (`after_turn` or `soon`); `passive` is an
invalid request.

```json
{"id": 7, "method": "thread/externalInput/submit", "params": {
  "version": 1, "ownerId": "cutex-owner", "threadId": "0199c0de-...",
  "runtimeGeneration": 3,
  "message": {"id": "ask-42.1", "source": {"kind": "agent", "id": "reviewer"},
              "type": "ASK", "delivery": "after_turn", "text": "..."},
  "view": null, "semanticSha256": "<64 hex>",
  "admission": "exclusive_turn"}}
```

Accepted:

```json
{"id": 7, "result": {
  "version": 1, "ownerId": "cutex-owner", "threadId": "0199c0de-...",
  "runtimeGeneration": 3,
  "statuses": [{"messageId": "ask-42.1", "semanticSha256": "<64 hex>",
                "deliveryState": "pending", "receipt": null,
                "processing": {"state": "pending", "attemptId": null, "reason": null}}],
  "turnId": "0199c0df-..."}}
```

Refused (JSON-RPC invalid request, nothing queued, no turn started):

```json
{"id": 7, "error": {"code": -32600,
  "message": "ExternalInput exclusive admission refused: busy (active_turn)",
  "data": {"externalInputError": "busy", "reason": "active_turn"}}}
```

| `reason` | Meaning |
|---|---|
| `active_turn` | A turn is running or reserved. |
| `pending_obligation` | A committed active item is released and waiting for its processing turn (it would be claimed by any next turn). Held items do not count. |
| `pending_turn_trigger` | Native mailbox input is waiting to start a turn. |
| `gate_closed` | The interruption gate is closed. Reopen it with `resume` if that is the owner's intent. |
| `plan_mode` | Automatic turns do not run in Plan mode. |
| `server_draining` | The host is draining. |

Rules:

- **Atomic.** Duplicate check, gate, obligations and the idle reservation are
  one decision under the ExternalInput runtime lock, in the same lock order as
  ordinary admission and idle reservation. The item is bound to the reserved
  turn before any lock is released.
- **One item per turn.** The reserved turn consumes the bound item and passive
  items, and no other active pending item at any boundary (initial, drain,
  deferred), including `soon` items admitted while it runs. Such items wait
  and get their own automatic turn after it. Native `StartIfIdle` dispatch never
  counts a bound item as dispatch work, and it cannot start while the turn is
  reserved.
- **Passive notes.** Submit tells as `passive` with `admission` absent before
  the exclusive item. They join the exclusive turn (or the next turn if it is
  refused); their receipts name the turn they joined.
- **`turnId` is synchronous; the receipt follows.** The receipt is the durable
  context pair, written at the turn's first input boundary inside the turn (it
  must follow `TurnStarted` in history). Waiting for it in the request would
  hold the per-thread request queue, so a stop could not be sent during
  pre-turn compaction or MCP startup. Once context is persisted, the receipt's
  `turnId` is always the returned `turnId`; `thread/externalInput/statusChanged`
  announces it.
- **No turn, no item.** If the turn ends before it writes the pair (interrupt,
  pre-turn compaction failure, a replaced reservation), the item is dropped
  from the volatile pending list, `statusChanged` fires and its status becomes
  `unknown`. Native never runs it later. Admission stays volatile, as before.
- **Replay.** Submitting a known message (same id and digest) never starts a
  second turn. The response carries its current status and its turn:
  `receipt.turnId` once persisted, the bound turn while pending, none for an
  item that was queued normally.
- Owner steering (`turn/start` or `turn/steer` on the running turn) still
  steers into it; that is cutex's "add to this ask" path. Native multi-agent
  mailbox delivery is unchanged; cutex launches do not use it.

## N2: gate control

The ExternalInput interruption gate is the durable `DispatchGate` fact. Today
every `turn/interrupt` (core `Op::Interrupt`) closes it, and only a human
`turn/start` or the per-message `retry` reopens it.

### `thread/externalInput/resume`

Reopens the gate without a human turn and dispatches released pending work.

```json
{"id": 8, "method": "thread/externalInput/resume", "params": {
  "version": 1, "ownerId": "cutex-owner", "threadId": "0199c0de-...",
  "runtimeGeneration": 3}}
```

```json
{"id": 8, "result": {"version": 1, "ownerId": "cutex-owner",
  "threadId": "0199c0de-...", "runtimeGeneration": 3, "reopened": true}}
```

`reopened: false` means the gate was already open (no record written). It
writes the same `DispatchGate { paused: false }` record a human turn writes,
so history stays readable by older binaries. Held items (`interrupted`,
`request_uncertain`, `no_output`) stay held; only `retry` releases them.

### `thread/externalInput/interrupt`

Interrupts the named active turn **without** closing the gate (requester
stop). It responds `{}` once the turn is aborted, like `turn/interrupt`.

```json
{"id": 9, "method": "thread/externalInput/interrupt", "params": {
  "version": 1, "ownerId": "cutex-owner", "threadId": "0199c0de-...",
  "runtimeGeneration": 3, "turnId": "0199c0df-..."}}
```

If that turn is not active: error `-32600`, `"turn <id> is not active"`.
Claimed items of the aborted turn end `held(request_uncertain)` as with any
interrupt. Other pending work is dispatched afterwards. `turn/interrupt` is
unchanged; it is a separate method rather than a `turn/interrupt` parameter so
upstream `TurnInterruptParams` (constructed in ~18 places) stays untouched and
only the bound owner can use it. The response type is
`ExternalInputInterruptResponse` (`{}`); sharing `TurnInterruptResponse` with an
experimental method would drop that type from the stable schema exports.

## For cutex

1. Read `externalInputControlVersion` from `initialize`; without it, keep the
   ask path disabled (or fall back to today's behaviour with `sharedTurn`
   flagging).
2. Write the attempt record (message id) before submitting. Submit with
   `admission: "exclusive_turn"`. On success, bind attempt to `turnId`
   immediately. On `busy`, the ask stays in the cutex queue; `gate_closed`
   means an interrupt from some path closed the gate (show `heldByRuntime` or
   `paused(owner)`, resume only on the owner's resume).
3. "Handled" is still the `output_observed` processing phase for the message
   id; the receipt's `turnId` must equal the bound `turnId`.
4. Crash recovery: query `thread/externalInput/status` by message id. `unknown`
   means the item never reached context: resubmitting is safe. A receipt means
   it is bound to `receipt.turnId`. A held state means the attempt was
   interrupted (`interrupted{...}` in the design).
5. Requester stop: `thread/externalInput/interrupt` with the ask's turn id.
   Owner stop: keep using `turn/interrupt`; owner resume calls
   `thread/externalInput/resume` together with the cutex queue resume.
6. Types: `ExternalInputSubmitParams.admission`, `ExternalInputAdmission`,
   `ExternalInputResponse.turnId`, `ExternalInputResumeParams/Response`,
   `ExternalInputInterruptParams/Response` (experimental exports), and
   `InitializeResponse.externalInputControlVersion` (stable exports). The
   busy reasons are `codex_protocol::external_input_status::BusyReason`
   (`ExternalInputBusyReason`); they appear only in error data, so they are
   not in the generated bundles.

## Generated files

- Stable fixtures (checked in): `codex-rs/app-server-protocol/schema/typescript/`
  (`InitializeResponse.ts`, `v2/ExternalInputAdmission.ts`, `v2/index.ts`) and
  `schema/json/` (`v1/InitializeResponse.json`, the two bundled
  `codex_app_server_protocol*.schemas.json`).
- Experimental methods and params live only in
  `schema/precomputed/app-server-exports-experimental.json.zst`. A client gets
  them with `codex app-server generate-ts --experimental` (and
  `generate-json-schema --experimental`) from a binary built from this branch.
- Regenerate: `CODEX_APP_SERVER_SCHEMA_ROOT=<abs>/codex-rs/app-server-protocol/schema
  CODEX_APP_SERVER_SCHEMA_EXPERIMENTAL=0|1 cargo test -p codex-app-server-protocol
  --lib schema_fixtures_tests::write_schema_fixtures_from_env -- --ignored --exact`
  (what `just write-app-server-schema` runs, without its Python SDK step; the
  fork has never regenerated `sdk/python` for ExternalInput fields).

## Spike: could the cutex per-session lease alone make admission atomic?

Question (design §12.2): with today's APIs, can cutex's admission lease make
"one ask per turn, bound to that turn, crash-safe" hold without N1?

**No.** The lease serialises what cutex itself starts, but admission is not a
single decision anywhere with today's API:

1. **Check-then-submit is not atomic.** cutex can only observe "idle" and then
   submit. Native turn sources outside the lease can start a turn in between:
   the goal extension (`ext/goal`), the native queue (`ext/queue`,
   `thread/queue/*`), `ext/agent`, memory writing, mailbox trigger mail
   (`maybe_start_turn_for_pending_work`), other app-server clients. A `soon`
   item then joins that running turn; an `after_turn` item is drained by the
   next turn whatever starts it.
2. **The turn boundary drains everything.** `consume_external_input` loops over
   all eligible pending items and gives each a receipt with the same turn id;
   `StartIfIdle` dispatch reserves an automatic turn for "the pending work",
   not for one item. The lease can keep cutex from submitting two items, but
   not keep native from combining one with items native admitted itself.
3. **A refused dispatch strands the item.** Submit admits first and dispatches
   after. If dispatch is refused (gate closed, Plan mode, draining, pending
   trigger mail), the item stays in the native pending list. There is no
   withdraw API, so it runs inside whatever turn starts next, typically the
   owner's own `turn/start`, which also reopens the gate: a shared turn,
   answer captured from the owner's reply.
4. **The gate is invisible and one-way.** No API reports the gate (only a held
   item's status does, after the fact), and only a human turn reopens it. A
   requester stop through `turn/interrupt` closes it for every requester.
5. **Binding is late.** The turn id is only known after the durable pair is
   written (receipt). Between submit and receipt, cutex cannot target a stop at
   the ask's turn, nor tell "queued natively, will run later" from "about to
   start".
6. **Crash recovery is not atomic with native.** Pending admissions are
   volatile (lost on runtime restart: fine, resubmit), but committed `pending`
   obligations survive restart and are claimed by the next turn of any origin.
   cutex can reconcile with `status` by message id, but cannot stop a native
   turn source from starting a turn that claims the obligation before it does.

With N1 the decision is one critical section: either refused (nothing queued,
nothing to withdraw) or reserved and bound to a turn id returned to the
caller; a turn that ends before writing the pair drops the item, so
`status == unknown` after any crash means "never delivered, safe to resubmit"
and a receipt always names the bound turn. N1 is therefore required (P1).

## Implementation notes

- Core: `core/src/session/external_input_exclusive.rs` (admission, reservation,
  release), `external_input/{mod,eligibility,api}.rs`,
  `session/external_input.rs` (boundary filter), `session/turn_input.rs`
  (`start_reserved_external_input_turn`), `tasks/mod.rs` (release on turn end
  and abort). `external_input_gate` now reports whether the gate changed.
- No rollout record or sqlx migration changes. Resume reuses the existing
  `DispatchGate` record; exclusivity is volatile runtime state.
- Tests: `core/src/session/external_input_exclusive_tests.rs` (refusals queue
  nothing; boundary predicates; release), `core/tests/suite/external_input_exclusive.rs`
  (end to end on a bound, resumed thread with a mock model),
  `app-server-protocol` serde tests for the new params.
