# Antigravity IDE Integration & Tesie Gateway Debugging Handoff

## 1. Executive Summary & Goal
The objective is to route Google Antigravity IDE (version 2.33.0, running on Windows 11) transparently through **Tesie** (`http://127.0.0.1:20129`) without needing root/admin DNS redirection (`hosts` file loopback) or self-signed Root CA SSL interception.

Antigravity natively supports an environment variable hook:
```powershell
$env:CLOUD_CODE_URL = "http://127.0.0.1:20129"
```
When this variable is set, Antigravity's core backend daemon (`language_server.exe`) overrides its internal `CloudCodeServerURL` (`https://daily-cloudcode-pa.googleapis.com`) and sends all `/v1internal:*` requests directly to `http://127.0.0.1:20129`.

The goal is for Antigravity to:
1. Accept the local companion project and tier without account/login rejection.
2. Populate its model selection dropdown with all 53 Tesie models and combos (Elephant, GoldFish, DeepSeek V4.1 Flash, Claude Opus 5.5, Gemini 4 Argon, OpenCode Free, etc.).
3. Intercept `streamGenerateContent` so that all code generation and chat requests are converted from Google's format to OpenAI/combo format, execute via Tesie's proxy pool/providers, and stream back via Server-Sent Events (SSE).

---

## 2. Antigravity Architecture & Internal Binary Details
- **Main GUI Executable**: `C:\Users\Mushf\AppData\Local\Programs\antigravity\Antigravity.exe` (Electron app based on VS Code)
- **Language Server Binary**: `C:\Users\Mushf\AppData\Local\Programs\antigravity\resources\bin\language_server.exe` (Compiled Go binary, ~100MB)
- **Log Files**:
  - `C:\Users\Mushf\AppData\Roaming\antigravity\logs\language_server.log`
  - `C:\Users\Mushf\AppData\Roaming\antigravity\logs\main.log`
- **Internal Secrets / Strings Discovered in `language_server.exe`**:
  - `Overriding CloudCodeServerURL via CLOUD_CODE_URL environment variable: %q`
  - `[AuthProvider] UpdateEndpointURL called with CLOUD_CODE_URL: %q`
  - `[AuthProvider] SetProjectID called with projectID: %q`
  - Uses Go's strict `google.golang.org/protobuf/encoding/protojson` package to unmarshal JSON responses.

---

## 3. Endpoints Called by Antigravity in Order

### 1. `POST /v1internal:loadCodeAssist`
- **When Called**: At initial boot to determine project ID, tier, and whether the account is eligible.
- **Protobuf Definition**: `v1internal.LoadCodeAssistResponse`
- **Required JSON Schema**:
  ```json
  {
    "cloudaicompanionProject": "projects/tesie-companion",
    "currentTier": {
      "id": "g1-pro-tier",
      "name": "Google AI Pro"
    },
    "paidTier": {
      "id": "g1-pro-tier",
      "name": "Google AI Pro"
    },
    "allowedTiers": [
      {
        "id": "g1-pro-tier",
        "name": "Google AI Pro",
        "isDefault": true
      }
    ],
    "ineligibleTiers": []
  }
  ```
  *(CRITICAL: Do NOT emit `"cloudaicompanion_project"` snake_case alongside `"cloudaicompanionProject"`. `protojson` rejects duplicate field definitions).*
  *(Project ID is the fully-qualified `projects/tesie-companion` form on every endpoint. Earlier builds returned a bare `tesie-companion` here and the prefixed form everywhere else; the bare form is no longer used.)*

### 2. `POST /v1internal:fetchUserInfo`
- **When Called**: During account initialization and profile sync.
- **Protobuf Definition**: `v1internal.FetchUserInfoResponse`
- **Required JSON Schema**:
  ```json
  {
    "userSettings": {
      "userTier": {
        "id": "g1-pro-tier",
        "name": "Google AI Pro"
      }
    },
    "userTags": [],
    "regionCode": "US"
  }
  ```

### 3. `POST /v1internal:onboardUser`
- **When Called**: If the project requires onboarding.
- **Protobuf Definition**: `google.longrunning.Operation` containing `v1internal.OnboardUserResponse`
- **Required JSON Schema**:
  ```json
  {
    "done": true,
    "response": {
      "cloudaicompanionProject": {
        "id": "projects/tesie-companion"
      }
    }
  }
  ```

### 4. `POST /v1internal:listCloudAICompanionProjects`
- **When Called**: To discover available projects.
- **Required JSON Schema**:
  ```json
  {
    "projects": [
      {
        "id": "projects/tesie-companion",
        "name": "Tesie Companion Project"
      }
    ]
  }
  ```

### 5. `POST /v1internal:getCodeAssistGlobalUserSetting`
- **Required JSON Schema**:
  ```json
  {
    "cloudaicompanionProject": "projects/tesie-companion"
  }
  ```

### 6. `POST /v1internal:fetchAvailableModels`
- **When Called**: To populate the IDE model dropdown.
- **Protobuf Definition**: `v1internal.FetchAvailableModelsResponse` — map values are `v1internal.ModelDetails`.
- **Required JSON Schema**:
  ```json
  {
    "defaultAgentModelId": "gemini-3.8-flash",
    "models": {
      "gemini-4-argon": {
        "displayName": "Gemini 4 Argon",
        "model": "MODEL_GOOGLE_GEMINI_2_5_PRO",
        "apiProvider": "API_PROVIDER_GOOGLE_GEMINI",
        "modelProvider": "MODEL_PROVIDER_GOOGLE",
        "maxTokens": 1048576,
        "maxOutputTokens": 65536,
        "supportsImages": true,
        "supportsThinking": true,
        "quotaInfo": { "remainingFraction": 1.0, "resetTime": "2030-01-01T00:00:00Z" }
      },
      "claude-opus-5-5": {
        "displayName": "Claude Opus 5.5",
        "model": "MODEL_CLAUDE_4_OPUS",
        "apiProvider": "API_PROVIDER_ANTHROPIC_VERTEX",
        "modelProvider": "MODEL_PROVIDER_ANTHROPIC",
        "maxTokens": 1048576,
        "maxOutputTokens": 65536,
        "supportsImages": true,
        "quotaInfo": { "remainingFraction": 1.0, "resetTime": "2030-01-01T00:00:00Z" }
      },
      "GoldFish": {
        "displayName": "Combo: GoldFish",
        "model": "MODEL_GOOGLE_GEMINI_2_5_PRO",
        "apiProvider": "API_PROVIDER_GOOGLE_GEMINI",
        "modelProvider": "MODEL_PROVIDER_GOOGLE",
        "quotaInfo": { "remainingFraction": 1.0, "resetTime": "2030-01-01T00:00:00Z" }
      }
    },
    "agentModelSorts": [
      {
        "displayName": "Tesie",
        "groups": [{ "displayName": "Tesie Models", "modelIds": ["gemini-4-argon", "claude-opus-5-5", "GoldFish"] }]
      }
    ]
  }
  ```

Two fields are load-bearing and were the actual cause of "No Models Available" (see Bug 5):

- **`model`** — the `exa.codeium_common_pb.Model` enum for the entry. It does NOT come from the map key; an entry without it never becomes a `ClientModelConfig` and is invisible to the picker. Valid literals are compiled into `language_server.exe` (e.g. `MODEL_GOOGLE_GEMINI_2_5_PRO`, `MODEL_GOOGLE_GEMINI_2_5_FLASH`, `MODEL_GOOGLE_GEMINI_2_5_FLASH_LITE`, `MODEL_CLAUDE_4_OPUS`, `MODEL_CLAUDE_4_SONNET`, `MODEL_CLAUDE_4_5_HAIKU`, `MODEL_OPENAI_GPT_OSS_120B_MEDIUM`, `MODEL_DEEPSEEK_V3`, `MODEL_XAI_GROK_3`).
- **`agentModelSorts`** — the picker is built from these `ModelSort.groups[].modelIds`, not from the `models` map. No sorts means an empty picker even when every model parses correctly.

`available` is NOT a `ModelDetails` field; do not send it.

**Preferred source: pass the real list through.** `handle_antigravity_fetch_available_models` first calls
`tesie_providers::fetch_available_models_raw`, which replays the authenticated
`POST /v1internal:fetchAvailableModels` request against Google (same relay, headers and
`X-Client-Version` the quota refresh uses) and returns the payload verbatim. The synthesised
catalogue below is only a fallback for when no enabled account can reach Google. Verified on a
`ready` account: 36 genuine ids (`gemini-3.8-flash-high`, `gemini-4-neon-high`, `gemini-4-carbon-disabled`,
`claude-opus-5-5-high`, …) with `defaultAgentModelId` taken from upstream. Prefer this — invented ids
become `antigravity/<invented>` and fail with `No quota available for <name>`, which is exactly the trap
the old `default_target_model` workaround papered over.

Note `available` **is** a real upstream model field (see `refresh_account_quota`, which reads
`models[*].available` / `disabled` / `isLocked` / `isInternal`); it is simply not part of the
`ModelDetails` message we emit when synthesising.

### 7. `POST /v1internal:retrieveUserQuotaSummary`
- **When Called**: To check remaining quota bars.
- **Required JSON Schema**:
  ```json
  {
    "groups": [
      {
        "displayName": "Antigravity Models",
        "buckets": [
          {
            "bucketId": "antigravity-weekly",
            "displayName": "Weekly Limit",
            "window": "weekly",
            "remainingFraction": 1.0,
            "resetTime": "2030-01-01T00:00:00Z"
          },
          {
            "bucketId": "antigravity-5h",
            "displayName": "Session Limit",
            "window": "5h",
            "remainingFraction": 1.0,
            "resetTime": "2030-01-01T00:00:00Z"
          }
        ]
      }
    ]
  }
  ```

### 8. `POST /v1internal:streamGenerateContent?alt=sse`
- **When Called**: When user sends a chat message, asks for code completion, or runs agent loops.
- **Payload**: Gemini/CloudCode chat payload (`{ contents: [...], generationConfig: {...}, model: "..." }`).
- **Gateway Behavior**:
  - `normalize_antigravity_request_to_openai()` converts contents, roles (`user`, `model` -> `assistant`), images/inlineData, and functionCalls/functionResponses into standard OpenAI chat completion JSON.
  - Passes request to Tesie's router / provider engine / combo aggregator with `bypass_auth: true`.
  - Maps streaming OpenAI SSE chunks (`data: {"choices":[{"delta":{"content":"..."}}]}`) into Google Cloud Code SSE chunks (`data: {"response":{"candidates":[{"content":{"parts":[{"text":"..."}]}}]}}`).

---

## 4. History of Bugs & Root Causes

### Bug 1: `connectex: No connection could be made because target machine actively refused it`
- **Symptom**: Antigravity dialog: `failed to get load code assist response: Post "http://127.0.0.1:20129/v1internal:loadCodeAssist": dial tcp 127.0.0.1:20129: connectex: No connection could be made...`
- **Root Cause**: The Tesie gateway background process had stopped or crashed, leaving port 20129 closed.
- **Fix**: Launch Tesie with daemon supervisor (`& "$env:LOCALAPPDATA\Tesie\bin\tesie.exe" --serve --port 20129`).

### Bug 2: `There was an unexpected issue setting up your account. request failed with empty body (code 405)`
- **Symptom**: Antigravity popped up an error and logged the account out.
- **Root Cause**: In `backend/crates/tesie/src/lib.rs`, requests to routes not matched by the gateway fell through to `serve_spa_asset` (the dashboard frontend file server). The original guard checked the HTTP **method before the path**, so any `POST` — including to `/v1internal:*` endpoints — was rejected with `StatusCode::METHOD_NOT_ALLOWED` (HTTP 405) and an empty body. Static file servers only accept `GET`/`HEAD`, so this fired for every unhandled POST.
- **Fix**:
  1. Reordered `serve_spa_asset` so the API-shaped path guard (`/api`, `/v1`, `/v1internal`) runs **first** and returns `404 {"error":"endpoint not found"}`. The method check now only applies to genuine static-asset paths. This is the actual fix for the empty-body 405 — the path guard alone did not help while it sat behind the method check.
  2. Implemented route handlers for `fetchUserInfo`, `onboardUser`, `listCloudAICompanionProjects`, `getCodeAssistGlobalUserSetting`, `fetchAdminControls`, etc. in `backend/crates/gateway/src/lib.rs`.
- **Verified**: `POST /v1internal:doesNotExist` -> 404, `POST /api/unknown` -> 404, `POST /nonsense` -> 405 (correct: non-API path with wrong method), `GET /` -> 200.

### Bug 3: `Sorry, this account is ineligible to use Antigravity. This client is no longer supported for Gemini Code Assist for individuals...`
- **Symptom**: Antigravity showed a blocking screen refusing the account.
- **Root Cause**: We implemented a passthrough forwarding `loadCodeAssist` upstream to `https://daily-cloudcode-pa.googleapis.com` with the user's bearer token. Google recently deprecated "Gemini Code Assist for individuals" for personal Gmail accounts, so Google's backend returned an `ineligibleTiers` array with that exact deprecation text.
- **Fix**: Removed the upstream passthrough for `loadCodeAssist` and `fetchUserInfo`. Tesie now directly mocks an active `g1-pro-tier` with `ineligibleTiers: []`. The old `try_antigravity_passthrough` helper has been deleted from `gateway/src/lib.rs`, so there is no dormant code path that re-enables the Google round-trip by accident.

### Bug 4: `proto: (line 1:124): duplicate field "cloudaicompanion_project"`
- **Symptom**: Antigravity dialog: `There was an unexpected issue setting up your account. proto: (line 1:124): duplicate field "cloudaicompanion_project"`.
- **Root Cause**: In `handle_antigravity_load_code_assist`, the response payload contained both `"cloudaicompanionProject"` (camelCase) and `"cloudaicompanion_project"` (snake_case). Go's `protojson` unmarshaler maps both to field tag 3 and strictly errors when duplicate JSON keys map to the same protobuf field.
- **Fix**: Removed `"cloudaicompanion_project"`, keeping only the canonical camelCase `"cloudaicompanionProject"`. Also sanitized `fetchAvailableModels` to remove `"default_agent_model_id"` and keep only `"defaultAgentModelId"`.

### Bug 5: `No Models Available: No models are available for your current project or account...`
- **Symptom**: The chat panel shows a `No Models Available` toast, the model selector shows `No Model Selected`, and the welcome screen never gets a model.
- **Root Cause**: The picker is NOT built from the `models` map of `fetchAvailableModels`. The UI reads `cascadeModelConfigData.clientModelConfigs` out of the local `LanguageServerService/GetUserStatus` RPC, and `language_server` only produces a `ClientModelConfig` for a model whose `ModelDetails.model` enum is set AND that is referenced by `agentModelSorts`. Our old payload sent neither (`displayName` + `available` only), so 53 models parsed successfully and produced zero picker entries — a 200 response with an empty list, no error message.
- **Diagnosis**: Read the RPC directly instead of screenshotting. `GetUserStatus` needs the `x-codeium-csrf-token` header (value from `window.__APP_CONFIG__.csrfToken`), and can be called from inside the webview via CDP (`--remote-debugging-port=9222`). A populated result looks like `cascadeModelConfigData.clientModelConfigs[]` with `label`, `modelOrAlias.model`, `quotaInfo` and `modelId` per entry. `GetCascadeModelConfigs` returns `{}` even when healthy — it is not the picker source.
- **Fix**: Emit `ModelDetails` (`displayName` + `model` enum + `apiProvider`/`modelProvider` + `quotaInfo`) and add `agentModelSorts` listing every model id. Verified end-to-end: picker renders "Gemini 4 Carbon" with no toast.

Separately: if the IDE shows `No Project ⌵` and zero traffic reaches Tesie, then Antigravity is talking to Google and no gateway-side fix can help — set the hook first.

**IMPORTANT — verify the hook is actually set before debugging anything else.** `CLOUD_CODE_URL` must exist in the environment of the shell/explorer that launches `Antigravity.exe`; setting it in a different terminal has no effect on an already-running IDE. Confirm with:

```powershell
[Environment]::GetEnvironmentVariable("CLOUD_CODE_URL","User")   # persisted, used by relaunches
Get-ChildItem Env:CLOUD_CODE_URL                                 # this shell only
```

If both are empty while `language_server.exe` is running, the IDE is talking to real Google and every symptom in this document is expected — no amount of gateway-side fixing will change it.

### Model resolution: dropdown entry -> actual provider route
`fetchAvailableModels` only populates the picker; it does not guarantee the selected entry resolves at generation time. The chain in `normalize_antigravity_request_to_openai` is:

1. Read `model` from the body (or `request.model`), strip a `models/` prefix.
2. Apply MITM config: explicit `model_aliases` entry, then combo-name match, then `default_target_model`.
3. Otherwise infer a route prefix: an existing `antigravity/`, `oc/`, `ovh/`, `combo/` prefix is kept; a name matching an enabled combo becomes `combo/<name>`; **everything else becomes `antigravity/<model>`**.

Step 3 is the failure point to watch. A model key that is in the dropdown but is neither a `combo/` target nor a real Antigravity provider model becomes `antigravity/<key>` and fails at `streamGenerateContent`, not when the list loads. When a model "appears but won't work", check whether that key exists in the Antigravity curated catalog or in `mitm.model_aliases`, and add an alias if it does not.

Concretely: the picker's default entry is a Tesie id (`gemini-4-carbon`) that resolves to `antigravity/gemini-4-carbon`, which the real backend has no quota for and answers with `403 insufficient_quota: No quota available for Carbon`. Set `default_target_model` to a real provider model to make every unaliased picker entry work:

```powershell
# PATCH /api/admin/mitm/config
{"default_target_model":"antigravity/gemini-2.5-flash"}
```

Verify with a real request, not the picker alone:

```powershell
curl.exe --noproxy "*" -s -X POST "http://127.0.0.1:20129/v1internal:streamGenerateContent?alt=sse" `
  -H "Content-Type: application/json" --data-binary "@body.json"
# expect: data: {"response":{"candidates":[{"content":{"parts":[{"text":"OK"}],"role":"model"}}]}}
```

### Generation fails with "No enabled Antigravity accounts connected"
That error is account state, not MITM — the picker can be fully populated while generation is dead. Check the authoritative flags, then enable a connection that has no ban or licence error:

```powershell
curl.exe --noproxy "*" -s "http://127.0.0.1:20129/api/admin/antigravity/accounts"
# PATCH /api/admin/connections/{id}   {"enabled":true,"reset_cooldown":true}
```

An account whose `last_error` says `Banned: ... Terms of Service` or `No active Gemini Code Assist license` cannot be rescued by enabling it — pick a different account or run the OAuth flow. A healthy one flips to `status: "ready"`, after which generation reaches the provider's quota logic instead of the account gate.

### Which picker ids the upstream actually serves
Verified against a `ready` account. Quota is **per model**, so a populated picker still fails per-entry:

| id | result |
|---|---|
| `gemini-3-flash`, `gemini-2.5-flash`, `gemini-2.5-flash-lite`, `gemini-4-argon` | `200` |
| `gemini-3.8-flash`, `gemini-4-neon`, `gemini-4-hydrogen` | `200` (small `maxOutputTokens` returns `MAX_TOKENS` with no text — not an error) |
| `gemini-4-carbon`, `claude-opus-5-5`, `claude-sonnet-5-5`, `gpt-oss-120b-medium` | `403 insufficient_quota` on this account |

Because the picker selects `gemini-4-carbon` by default, an unset `default_target_model` makes the first send fail with `No quota available for Carbon`. Note that `PATCH /api/admin/mitm/config` with `{"default_target_model": null}` is ignored — send `""` to clear it.

Once the picker serves the real upstream list, this stops mattering: the default entry is a genuine id
(`gemini-4-neon-high` on the reference account) and completes without any redirect. Keep
`default_target_model` empty; it masks per-model quota failures.

### What 9router does differently (and why we did not copy its transport)
`decolua/9router` and `luqman-v1/9router-go` intercept by redirecting the target hosts to
`localhost:443` in the system hosts file and terminating TLS with a generated root CA, intercepting only
`:generateContent` / `:streamGenerateContent` and passing every other endpoint through to Google. Their
model picker therefore shows real upstream models, which is the idea adopted here.

Their transport does not work on Antigravity 2.x: the `agy` agent embeds its own CA pool
(`NewCertPool` / `RootCAs`) and reads no TLS/CA/proxy env vars, so it fails the handshake with
`x509: certificate signed by unknown authority` while the UI still looks healthy and chat is billed to
the user's own account (their issue `decolua/9router#5597`). `CLOUD_CODE_URL` is read by the agent, so it
is the only hook that works on 2.33.0.

---

## 5. Key Source Code Locations
Line numbers below were verified against the working tree after the 405/404 reorder and the dead-code deletion; they will drift as the file changes, so grep the function name rather than trusting the number.

- **Gateway Router & Handlers**: `d:\Apps\TesRoute\backend\crates\gateway\src\lib.rs`
  - `gateway_router` — line 37 (all `/v1internal:*` routes, lines 51–86)
  - `handle_antigravity_load_code_assist` — line 187
  - `handle_antigravity_fetch_user_info` — line 225
  - `handle_antigravity_onboard_user` — line 248
  - `handle_antigravity_list_companion_projects` — line 269
  - `handle_antigravity_user_settings` — line 290
  - `handle_antigravity_retrieve_user_quota_summary` — line 368
  - `handle_antigravity_fetch_available_models` — line 478
  - `handle_antigravity_stream_generate_content` — line 582
  - `handle_antigravity_generate_content` — line 615
  - `normalize_antigravity_request_to_openai` — line 632
  - `map_openai_sse_to_gemini` — line 831
- **SPA Fallback Guard**: `d:\Apps\TesRoute\backend\crates\tesie\src\lib.rs` — `serve_spa_asset` at line 100 (path guard precedes method check)
- **Installed Production Binaries**:
  - `C:\Users\Mushf\AppData\Local\Tesie\bin\tesie.exe`
  - `C:\Users\Mushf\AppData\Local\Tesie\bin\tesie-daemon.exe`

---

## 6. How to Build & Test (Copy-Paste for Claude Code)

### Check if Tesie is running:
```powershell
Get-NetTCPConnection -LocalPort 20129 -ErrorAction SilentlyContinue | Select-Object LocalAddress, LocalPort, State, OwningProcess
```

### Build & Deploy Release Binary:
```powershell
cargo build --release --manifest-path backend/Cargo.toml -p tesie

# Stop old process and deploy
$conn = Get-NetTCPConnection -LocalPort 20129 -ErrorAction SilentlyContinue
if ($conn) { Stop-Process -Id $conn.OwningProcess -Force -ErrorAction SilentlyContinue }
Copy-Item -Path "backend/target/release/tesie.exe" -Destination "$env:LOCALAPPDATA\Tesie\bin\tesie.exe" -Force
Copy-Item -Path "backend/target/release/tesie-daemon.exe" -Destination "$env:LOCALAPPDATA\Tesie\bin\tesie-daemon.exe" -Force

# Start background server
Start-Process -FilePath "$env:LOCALAPPDATA\Tesie\bin\tesie.exe" -ArgumentList "--serve --port 20129" -WindowStyle Hidden
```

### Test Endpoints via cURL:
```powershell
# 1. loadCodeAssist
curl.exe --noproxy "*" -s -X POST "http://127.0.0.1:20129/v1internal:loadCodeAssist" -H "Content-Type: application/json" -d "{}"

# 2. fetchUserInfo
curl.exe --noproxy "*" -s -X POST "http://127.0.0.1:20129/v1internal:fetchUserInfo" -H "Content-Type: application/json" -d "{}"

# 3. fetchAvailableModels
curl.exe --noproxy "*" -s -X POST "http://127.0.0.1:20129/v1internal:fetchAvailableModels" -H "Content-Type: application/json" -d "{}"

# 4. retrieveUserQuotaSummary
curl.exe --noproxy "*" -s -X POST "http://127.0.0.1:20129/v1internal:retrieveUserQuotaSummary" -H "Content-Type: application/json" -d "{}"
```

### Launch Antigravity Hooked to Tesie:
```powershell
$env:CLOUD_CODE_URL = "http://127.0.0.1:20129"
& "$env:LOCALAPPDATA\Programs\antigravity\Antigravity.exe" .
```

### Rollback (Direct Google Connection):
```powershell
Remove-Item env:CLOUD_CODE_URL -ErrorAction SilentlyContinue
[Environment]::SetEnvironmentVariable("CLOUD_CODE_URL", $null, "User")
& "$env:LOCALAPPDATA\Programs\antigravity\Antigravity.exe" .
```
