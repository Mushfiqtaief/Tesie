# Codebuff Native Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and integrate a native Codebuff provider into Tesie with multi-account GitHub OAuth, a 5-session daily quota tracker with live 1-hour countdown timers, and automated multi-account load balancing and fallback.

**Architecture:** Implement `tesie-providers::codebuff` entirely in Rust to handle GitHub OAuth polling, Codebuff 1-hour session lifecycle management, model binding, and run-chain streaming. Store account connections and session counters in SQLite. Expose multi-account management in `ProvidersView.tsx` and real-time 5-session gauges in `QuotaTrackerView.tsx`.

**Tech Stack:** Rust (Tokio, Reqwest, Serde, SQLite), React 18, TypeScript, Tailwind CSS, SSE streaming.

**Spec:** [docs/superpowers/specs/2026-11-22-codebuff-provider-design.md](file:///d:/Apps/TesRoute/docs/superpowers/specs/2026-11-22-codebuff-provider-design.md)

## Global Constraints

- **Zero Subprocess Dependencies**: All logic must be compiled natively into the Tesie binary; no external Node.js or Bun processes.
- **SQLite Storage**: Use existing `connections`, `provider_configs`, and quota tables without breaking Antigravity, OpenCode, or OVH records.
- **Session Lifespan**: Upstream sessions last exactly 60 minutes. Use a 5-second safety buffer before expiry.
- **Rate-Limit Handling**: Backoff and rotate accounts on upstream 429 or session limits.
- **Strict File Links**: Use `file:///` format for all markdown links.

## Review Focus

1. **Session Expiry Handshake**: When an active 1-hour session expires while a request is in flight, the provider must transparently acquire a fresh session without failing the user request.
2. **Model Locking**: Codebuff binds a 1-hour session to a specific model. If a request for a different model arrives, the provider must safely end/recreate the session or route to an account already bound to that model.
3. **Midnight UTC Rollover**: Daily limited session usage (5 per day) must reset cleanly across calendar days.
4. **Account Fallback**: When Account A hits its 5-session daily limit on a limited-tier model, subsequent requests must automatically route to Account B.
5. **Stream Interruption Safety**: Run-chain finalization (`POST /api/v1/runs/finalize`) must execute even if the client disconnects mid-stream.

---

### Task 1: Core Domain Models & Database Store

**Files:**
- Modify: `backend/crates/core/src/models.rs:20-50`
- Modify: `backend/crates/store/src/lib.rs:80-120`
- Modify: `backend/crates/store/src/quotas.rs:1-80`
- Test: `backend/crates/core/tests/codebuff_models_test.rs`

- [x] **Step 1: Write the failing unit test for Codebuff core models**
  Create `backend/crates/core/tests/codebuff_models_test.rs` asserting `ProviderKind::Codebuff` serialization (`"codebuff"`), `CodebuffAccountQuota`, and `CodebuffActiveSessionInfo` structures.
- [x] **Step 2: Run test to confirm failure**
  Run `cargo test -p tesie-core --test codebuff_models_test` and verify compilation failure on missing enum variant.
- [x] **Step 3: Implement core models**
  Update `backend/crates/core/src/models.rs` to add:
  - `ProviderKind::Codebuff`
  - Display name and canonical ID mappings for Codebuff.
  - `CodebuffAccountQuota` and `CodebuffActiveSessionInfo`.
  - `CodebuffQuotaSnapshot`.
- [x] **Step 4: Extend store for Codebuff sessions**
  Update `backend/crates/store/src/quotas.rs` to track Codebuff daily session counts and active session instance data per account.
- [x] **Step 5: Run tests and commit**
  Run `cargo test -p tesie-core` and `cargo test -p tesie-store` to verify passes.
  Commit: `feat(core): add Codebuff provider kind, quota models, and store schema`.

---

### Task 2: Codebuff OAuth & Multi-Account Management

**Files:**
- Create: `backend/crates/providers/src/codebuff/oauth.rs`
- Modify: `backend/crates/providers/src/codebuff/mod.rs`
- Test: `backend/crates/providers/tests/codebuff_oauth_test.rs`

- [x] **Step 1: Write failing test for OAuth start and status parsing**
  Create `backend/crates/providers/tests/codebuff_oauth_test.rs` testing parsing of `/api/code` and `/api/status` responses from Freebuff/Codebuff OAuth.
- [x] **Step 2: Run test to confirm failure**
  Run `cargo test -p tesie-providers --test codebuff_oauth_test`.
- [x] **Step 3: Implement OAuth handshake and account creation**
  In `backend/crates/providers/src/codebuff/oauth.rs`:
  - `start_oauth(client)`: Calls `https://freebuff.llm.pm/api/code` $\to$ returns `{ fingerprintId, loginUrl }`.
  - `poll_oauth(client, fingerprint_id)`: Calls `https://freebuff.llm.pm/api/status` $\to$ returns auth token and user profile once GitHub login completes.
  - `add_account_by_token(store, token)`: Validates token against Codebuff and stores it in `connections` table as `ProviderKind::Codebuff`.
  - `list_accounts(store)`: Returns all active Codebuff accounts.
- [x] **Step 4: Run tests and verify**
  Run `cargo test -p tesie-providers --test codebuff_oauth_test`.
  Commit: `feat(providers): implement Codebuff OAuth handshake and account store`.

---

### Task 3: Codebuff Session Lifecycle & 5-Session Daily Quota Engine

**Files:**
- Create: `backend/crates/providers/src/codebuff/session.rs`
- Modify: `backend/crates/providers/src/codebuff/mod.rs`
- Test: `backend/crates/providers/tests/codebuff_session_test.rs`

- [x] **Step 1: Write failing test for 1-hour session expiry and daily 5-session limit**
  Create `backend/crates/providers/tests/codebuff_session_test.rs` verifying:
  - Session expires after 3600 seconds.
  - Daily limited count increments on new session creation and caps at 5.
  - Account selection falls back to second account when first account hits 5 sessions.
  - Midnight UTC rollover resets session count.
- [x] **Step 2: Run test to confirm failure**
  Run `cargo test -p tesie-providers --test codebuff_session_test`.
- [x] **Step 3: Implement CodebuffSessionManager**
  In `backend/crates/providers/src/codebuff/session.rs`:
  - `acquire_session(account, model)`: Checks cached session. If expired or locked to a different model, calls `POST /api/v1/freebuff/session`.
  - `record_session_usage(account, tier)`: Increments daily session count if `accessTier == "limited"`.
  - `select_account_for_model(model)`: Picks active account with lowest used daily sessions; skips accounts at 5/5 for limited models.
  - `get_quota_snapshot()`: Compiles real-time quotas and active session timers for all accounts.
- [x] **Step 4: Run tests and verify**
  Run `cargo test -p tesie-providers --test codebuff_session_test`.
  Commit: `feat(providers): implement Codebuff session lifecycle and 5-session daily quota engine`.

---

### Task 4: Codebuff Catalog & Upstream Execution Engine

**Files:**
- Create: `backend/crates/providers/src/codebuff/catalog.rs`
- Create: `backend/crates/providers/src/codebuff/executor.rs`
- Modify: `backend/crates/providers/src/lib.rs`
- Test: `backend/crates/providers/tests/codebuff_executor_test.rs`

- [x] **Step 1: Write failing test for catalog and payload formatting**
  Create `backend/crates/providers/tests/codebuff_executor_test.rs` verifying:
  - Catalog models (`deepseek-v4-pro`, `deepseek-v4-flash`, `minimax-m3`, `kimi-k2.6`, etc.) and tier flags.
  - Request payload structure (`codebuff_metadata`, `freebuff_instance_id`, `run_id`, `client_id`).
- [x] **Step 2: Run test to confirm failure**
  Run `cargo test -p tesie-providers --test codebuff_executor_test`.
- [x] **Step 3: Implement catalog and executor**
  In `backend/crates/providers/src/codebuff/catalog.rs`:
  - Define static/dynamic model catalog with `context_length`, `max_output_tokens`, `is_limited`, `multimodal`.
  In `backend/crates/providers/src/codebuff/executor.rs`:
  - `start_run_chain(client, token, agent_id)` $\to$ `POST https://www.codebuff.com/api/v1/runs`.
  - `execute_chat(req)` $\to$ `POST https://www.codebuff.com/api/v1/chat/completions`.
  - Relay SSE chunks to client.
  - `finalize_run_chain(...)` on completion.
- [x] **Step 4: Run tests and verify**
  Run `cargo test -p tesie-providers --test codebuff_executor_test`.
  Commit: `feat(providers): implement Codebuff model catalog and run-chain streaming executor`.

---

### Task 5: Gateway Routing & Admin API Endpoints

**Files:**
- Modify: `backend/crates/gateway/src/aggregator.rs:180-260`
- Modify: `backend/crates/admin/src/lib.rs:700-800`
- Modify: `backend/crates/tesie/src/main.rs:100-160`
- Test: `backend/crates/tesie/tests/e2e_codebuff_provider.rs`

- [ ] **Step 1: Write failing integration test for Codebuff gateway routing and admin endpoints**
  Create `backend/crates/tesie/tests/e2e_codebuff_provider.rs` testing:
  - `GET /api/providers/codebuff/accounts`
  - `POST /api/providers/codebuff/oauth/start`
  - `GET /api/quotas/codebuff`
  - `POST /v1/chat/completions` with model `codebuff/deepseek-v4-flash`
- [ ] **Step 2: Run test to confirm failure**
  Run `cargo test -p tesie --test e2e_codebuff_provider`.
- [ ] **Step 3: Wire Codebuff into gateway and admin router**
  - In `backend/crates/gateway/src/aggregator.rs`: Route `codebuff/*` to `CodebuffEngine`.
  - In `backend/crates/admin/src/lib.rs`: Add routes for Codebuff account management and quota fetching.
  - In `backend/crates/tesie/src/main.rs`: Initialize `CodebuffEngine` and inject into app state.
- [ ] **Step 4: Run integration test and verify**
  Run `cargo test -p tesie --test e2e_codebuff_provider`.
  Commit: `feat(gateway): integrate Codebuff provider routing and admin API endpoints`.

---

### Task 6: Frontend Providers View (Multi-Account OAuth UI)

**Files:**
- Modify: `frontend/src/api.ts`
- Modify: `frontend/src/types.ts`
- Modify: `frontend/src/views/ProvidersView.tsx`

- [ ] **Step 1: Update frontend API client and types**
  In `frontend/src/types.ts` and `frontend/src/api.ts`:
  - Add types: `CodebuffAccount`, `CodebuffOAuthStartResponse`, `CodebuffOAuthStatusResponse`.
  - Add API functions: `getCodebuffAccounts()`, `deleteCodebuffAccount()`, `startCodebuffOAuth()`, `pollCodebuffOAuth()`, `addCodebuffToken()`.
- [ ] **Step 2: Build Codebuff Provider Card and Account List**
  In `frontend/src/views/ProvidersView.tsx`:
  - Add Codebuff card with logo and description.
  - Render connected GitHub accounts with avatar, email, status badge, and delete button.
- [ ] **Step 3: Build "+ Add GitHub Account" Modal**
  - Add modal with two tabs:
    - **Tab 1 (1-Click OAuth)**: Click "Login with GitHub" $\to$ opens login URL in browser $\to$ polls status until completed.
    - **Tab 2 (Manual Token)**: Paste auth token directly.
- [ ] **Step 4: Typecheck and verify**
  Run `npm run build` in `frontend/` to ensure clean TypeScript compilation.
  Commit: `feat(frontend): add Codebuff multi-account management and OAuth modal in ProvidersView`.

---

### Task 7: Frontend Quota Tracker (5-Session Daily Tracker & Timers)

**Files:**
- Modify: `frontend/src/api.ts`
- Modify: `frontend/src/types.ts`
- Modify: `frontend/src/views/QuotaTrackerView.tsx`
- Modify: `frontend/src/utils/modelCapabilities.ts`

- [ ] **Step 1: Add Codebuff Quota Types & API**
  Add `CodebuffQuotaSnapshot` and `fetchCodebuffQuotas()` in `frontend/src/types.ts` and `frontend/src/api.ts`.
- [ ] **Step 2: Build Codebuff Quotas Card in QuotaTrackerView**
  In `frontend/src/views/QuotaTrackerView.tsx`:
  - Render **Codebuff Quota Tracker** section.
  - For each account:
    - Display account email / GitHub handle.
    - **5-Session Daily Progress Gauge**: Visual bar showing `X / 5 sessions used today` (green $\to$ amber $\to$ red when at 5/5).
    - **Active Session Timer**: Live ticking countdown timer for current 1-hour session (`Active on DeepSeek V4 Pro: 41m remaining` or `Idle / Ready`).
- [ ] **Step 3: Register Codebuff models in capabilities**
  Update `frontend/src/utils/modelCapabilities.ts` with context and output limits for Codebuff models.
- [ ] **Step 4: Typecheck and verify**
  Run `npm run build` in `frontend/`.
  Commit: `feat(frontend): add 5-session daily quota tracker and live countdown timers in QuotaTrackerView`.

---

### Task 8: Full Verification, Binary Build, and Deployment

**Files:**
- Output: `C:\Users\Mushf\AppData\Local\Tesie\bin\tesie.exe`
- Output: `C:\Users\Mushf\AppData\Local\Tesie\www`

- [ ] **Step 1: Run full Rust test suite**
  Run `cd backend; cargo test --workspace` to ensure all tests across all crates pass.
- [ ] **Step 2: Compile release frontend**
  Run `cd frontend; npm run build; Copy-Item -Path dist\* -Destination "$env:LOCALAPPDATA\Tesie\www" -Recurse -Force`.
- [ ] **Step 3: Compile release backend binary**
  Run `cd backend; cargo build --release -p tesie`.
- [ ] **Step 4: Deploy binary and restart daemon**
  Kill running tesie process, copy `tesie.exe` to `$env:LOCALAPPDATA\Tesie\bin\tesie.exe`, and launch daemon on port 20129.
- [ ] **Step 5: Live verification**
  - Verify `/v1/models` includes `codebuff/*` models.
  - Verify `/quotas` loads Codebuff Quota Tracker cleanly.
  - Verify `/providers` shows Codebuff account management card.
  Commit: `chore: deploy updated Tesie with Codebuff native provider`.
