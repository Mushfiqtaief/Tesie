import { useState } from "react";
import {
  ArrowRight,
  Check,
  Copy,
  Eye,
  EyeOff,
  Key,
  Trash2,
} from "lucide-react";
import { TableSkeleton } from "../components/Skeleton";
import {
  Button,
  Card,
  EmptyState,
  INPUT_CLASS,
  MeterBar,
  Modal,
  SegmentedControl,
  StatusPill,
} from "../components/UiPrimitives";
import type {
  ApiKeyView,
  CreateApiKeyPayload,
  CreatedApiKeyResponse,
  UpdateApiKeyPayload,
} from "../types";
import { formatCompactNumber, formatTimeAgo } from "../utils/format";

type SnippetTab = "curl" | "python" | "typescript" | "anthropic";

export function EndpointKeysView({
  keys,
  sampleModel,
  loading,
  nowMs,
  onCreateKey,
  onUpdateKey,
  onDeleteKey,
  onOpenCliTools,
}: {
  keys: ApiKeyView[];
  sampleModel: string;
  loading: boolean;
  nowMs: number;
  onCreateKey: (payload: CreateApiKeyPayload) => Promise<CreatedApiKeyResponse>;
  onUpdateKey: (id: string, payload: UpdateApiKeyPayload) => Promise<void>;
  onDeleteKey: (id: string, name: string) => Promise<void>;
  onOpenCliTools: () => void;
}) {
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [snippetTab, setSnippetTab] = useState<SnippetTab>("curl");
  const [revealedIds, setRevealedIds] = useState<Record<string, boolean>>({});

  // Create API key modal & banner
  const [keyOpen, setKeyOpen] = useState(false);
  const [keyName, setKeyName] = useState("");
  const [keyReqLimit, setKeyReqLimit] = useState("");
  const [keyTokLimit, setKeyTokLimit] = useState("");
  const [savingKey, setSavingKey] = useState(false);
  const [createdKey, setCreatedKey] = useState<CreatedApiKeyResponse | null>(null);

  if (loading && keys.length === 0) {
    return <TableSkeleton rows={5} />;
  }

  function copyText(id: string, val: string) {
    void navigator.clipboard.writeText(val);
    setCopiedId(id);
    setTimeout(() => setCopiedId((cur) => (cur === id ? null : cur)), 1500);
  }

  async function handleKeySubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!keyName.trim()) return;
    setSavingKey(true);
    try {
      const created = await onCreateKey({
        name: keyName.trim(),
        quota_requests_limit: keyReqLimit.trim() ? Number(keyReqLimit) : null,
        quota_tokens_limit: keyTokLimit.trim() ? Number(keyTokLimit) : null,
      });
      setCreatedKey(created);
      setKeyName("");
      setKeyReqLimit("");
      setKeyTokLimit("");
      setKeyOpen(false);
    } finally {
      setSavingKey(false);
    }
  }

  const origin =
    typeof window !== "undefined" ? window.location.origin : "http://127.0.0.1:20129";
  const baseV1 = `${origin}/v1`;
  const activeRawKey =
    keys.find((k) => k.enabled && k.raw_key)?.raw_key ||
    (keys.length > 0 ? "sk-tesie-YOUR_KEY" : "sk-tesie-local");
  const modelExample = sampleModel || "oc/glm-5.3-flash-free";

  const endpointRows = [
    {
      id: "base",
      label: "OpenAI-Compatible Base URL",
      url: baseV1,
      protocol: "Base URL",
    },
    {
      id: "chat",
      label: "Chat Completions",
      url: `${baseV1}/chat/completions`,
      protocol: "POST",
    },
    {
      id: "responses",
      label: "OpenAI Responses",
      url: `${baseV1}/responses`,
      protocol: "POST",
    },
    {
      id: "messages",
      label: "Anthropic Messages",
      url: `${baseV1}/messages`,
      protocol: "POST",
    },
    {
      id: "models",
      label: "Models & Combos List",
      url: `${baseV1}/models`,
      protocol: "GET",
    },
  ];

  const snippets: Record<SnippetTab, string> = {
    curl: `curl ${baseV1}/chat/completions \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer ${activeRawKey}" \\
  -d '{
    "model": "${modelExample}",
    "stream": true,
    "messages": [{"role": "user", "content": "Explain consistent hashing in 3 lines"}]
  }'`,
    python: `from openai import OpenAI

client = OpenAI(
    base_url="${baseV1}",
    api_key="${activeRawKey}",
)

resp = client.chat.completions.create(
    model="${modelExample}",
    messages=[{"role": "user", "content": "Hello from Tesie!"}],
)
print(resp.choices[0].message.content)`,
    typescript: `import { createOpenAICompatible } from "@ai-sdk/openai-compatible";
import { generateText } from "ai";

const tesie = createOpenAICompatible({
  name: "tesie",
  baseURL: "${baseV1}",
  apiKey: "${activeRawKey}",
});

const { text } = await generateText({
  model: tesie("${modelExample}"),
  prompt: "Hello from Tesie!",
});`,
    anthropic: `curl ${baseV1}/messages \\
  -H "Content-Type: application/json" \\
  -H "anthropic-version: 2023-06-01" \\
  -H "x-api-key: ${activeRawKey}" \\
  -d '{
    "model": "${modelExample}",
    "max_tokens": 512,
    "messages": [{"role": "user", "content": "Hello via Anthropic Messages wire format"}]
  }'`,
  };

  return (
    <div className="space-y-5">
      {/* Endpoints + API Keys */}
      <div className="grid grid-cols-1 items-start gap-5 xl:grid-cols-12">
        <Card
          noPadding
          className="xl:col-span-5"
          title="API endpoints"
          actions={
            <button
              type="button"
              onClick={onOpenCliTools}
              className="inline-flex items-center gap-1 text-xs font-medium text-[var(--accent)] hover:underline"
            >
              Auto-configure CLIs
              <ArrowRight className="h-3.5 w-3.5" />
            </button>
          }
        >
          <div className="divide-y divide-[var(--border-subtle)]">
            {endpointRows.map((ep) => (
              <div
                key={ep.id}
                className="group flex items-center justify-between gap-3 px-5 py-3 transition-colors hover:bg-[var(--bg-hover)]/45"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-medium text-[var(--text-primary)]">
                      {ep.label}
                    </span>
                    <span className="font-mono text-[10px] text-[var(--text-muted)]">
                      {ep.protocol}
                    </span>
                  </div>
                  <div className="mt-0.5 truncate font-mono text-xs text-[var(--text-secondary)]">
                    {ep.url}
                  </div>
                </div>

                <button
                  type="button"
                  onClick={() => copyText(ep.id, ep.url)}
                  className="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                >
                  {copiedId === ep.id ? (
                    <Check className="h-3.5 w-3.5 text-[var(--success)]" />
                  ) : (
                    <Copy className="h-3.5 w-3.5" />
                  )}
                  {copiedId === ep.id ? "Copied" : "Copy"}
                </button>
              </div>
            ))}
          </div>
        </Card>

        <Card
          noPadding
          className="xl:col-span-7"
          title="Client API keys"
          subtitle={
            keys.some((k) => k.enabled)
              ? "Bearer authentication active"
              : "Open local access until you issue a key"
          }
          actions={
            <Button variant="primary" size="sm" onClick={() => setKeyOpen(true)}>
              <Key className="h-3.5 w-3.5" />
              Create API key
            </Button>
          }
        >
          {createdKey && (
            <div className="border-b border-[var(--border-subtle)] bg-[var(--success-soft)]/50 p-4">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <span className="text-xs font-semibold text-[var(--text-primary)]">
                  Created key: {createdKey.name}
                </span>
                <div className="flex items-center gap-1.5">
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={() => copyText("new-key", createdKey.raw_key)}
                  >
                    {copiedId === "new-key" ? (
                      <Check className="h-3.5 w-3.5" />
                    ) : (
                      <Copy className="h-3.5 w-3.5" />
                    )}
                    {copiedId === "new-key" ? "Copied" : "Copy key"}
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => setCreatedKey(null)}>
                    Dismiss
                  </Button>
                </div>
              </div>
              <div className="mt-2 select-all break-all rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2 font-mono text-xs text-[var(--text-primary)]">
                {createdKey.raw_key}
              </div>
            </div>
          )}

          {keys.length === 0 ? (
            <div className="p-5">
              <EmptyState
                icon={<Key className="h-5 w-5" />}
                title="Open local access"
                description={`Local clients can call ${baseV1} without a token right now. Create a key anytime for per-client budgets.`}
                action={
                  <Button variant="primary" size="sm" onClick={() => setKeyOpen(true)}>
                    <Key className="h-3.5 w-3.5" />
                    Create first key
                  </Button>
                }
              />
            </div>
          ) : (
            <div className="divide-y divide-[var(--border-subtle)]">
              {keys.map((k) => {
                const hasCap =
                  (k.quota_requests_limit && k.quota_requests_limit > 0) ||
                  (k.quota_tokens_limit && k.quota_tokens_limit > 0);
                const revealed = !!revealedIds[k.id];

                return (
                  <div key={k.id} className="px-5 py-3.5">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <div className="min-w-0 space-y-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-sm font-semibold text-[var(--text-primary)]">
                            {k.name}
                          </span>
                          <span className="rounded bg-[var(--bg-elevated)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--text-secondary)]">
                            {revealed && k.raw_key ? k.raw_key : `${k.key_prefix}••••••••`}
                          </span>
                          <StatusPill tone={k.enabled ? "success" : "neutral"} dot>
                            {k.enabled ? "Enabled" : "Disabled"}
                          </StatusPill>
                        </div>

                        <div className="flex flex-wrap items-center gap-2 text-[11px] text-[var(--text-muted)] tabular-nums">
                          <span>{k.quota_requests_used} req</span>
                          <span>&middot;</span>
                          <span>{formatCompactNumber(k.quota_tokens_used)} tok</span>
                          <span>&middot;</span>
                          <span>Last used {formatTimeAgo(k.last_used_at, nowMs)}</span>
                        </div>
                      </div>

                      <div className="flex items-center gap-1">
                        {k.raw_key && (
                          <>
                            <button
                              type="button"
                              onClick={() =>
                                setRevealedIds((prev) => ({ ...prev, [k.id]: !prev[k.id] }))
                              }
                              className="rounded-md p-1.5 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                              title={revealed ? "Hide key" : "Reveal key"}
                            >
                              {revealed ? (
                                <EyeOff className="h-3.5 w-3.5" />
                              ) : (
                                <Eye className="h-3.5 w-3.5" />
                              )}
                            </button>
                            <button
                              type="button"
                              onClick={() => copyText(k.id, k.raw_key || "")}
                              className="rounded-md p-1.5 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                              title="Copy API key"
                            >
                              {copiedId === k.id ? (
                                <Check className="h-3.5 w-3.5 text-[var(--success)]" />
                              ) : (
                                <Copy className="h-3.5 w-3.5" />
                              )}
                            </button>
                          </>
                        )}
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => void onUpdateKey(k.id, { enabled: !k.enabled })}
                        >
                          {k.enabled ? "Disable" : "Enable"}
                        </Button>
                        <button
                          type="button"
                          onClick={() => void onDeleteKey(k.id, k.name)}
                          className="rounded-md p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--danger-soft)] hover:text-[var(--danger)]"
                          title={`Delete ${k.name}`}
                          aria-label={`Delete ${k.name}`}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    </div>

                    {hasCap && (
                      <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
                        {k.quota_requests_limit ? (
                          <MeterBar
                            label="Requests budget"
                            used={k.quota_requests_used}
                            limit={k.quota_requests_limit}
                            tone="success"
                          />
                        ) : null}
                        {k.quota_tokens_limit ? (
                          <MeterBar
                            label="Tokens budget"
                            used={k.quota_tokens_used}
                            limit={k.quota_tokens_limit}
                            tone="accent"
                          />
                        ) : null}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </Card>
      </div>

      {/* SDK Snippets */}
      <Card
        title="SDK quickstart"
        actions={
          <div className="flex flex-wrap items-center gap-2">
            <SegmentedControl<SnippetTab>
              size="sm"
              value={snippetTab}
              onChange={setSnippetTab}
              options={[
                { value: "curl", label: "cURL" },
                { value: "python", label: "Python" },
                { value: "typescript", label: "AI SDK" },
                { value: "anthropic", label: "Anthropic" },
              ]}
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={() => copyText(`sdk-${snippetTab}`, snippets[snippetTab])}
            >
              {copiedId === `sdk-${snippetTab}` ? (
                <Check className="h-3.5 w-3.5 text-[var(--success)]" />
              ) : (
                <Copy className="h-3.5 w-3.5" />
              )}
              {copiedId === `sdk-${snippetTab}` ? "Copied" : "Copy"}
            </Button>
          </div>
        }
      >
        <pre className="overflow-x-auto rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-4 font-mono text-xs leading-relaxed text-[var(--text-secondary)]">
          {snippets[snippetTab]}
        </pre>
      </Card>

      {/* Create Key Modal */}
      <Modal open={keyOpen} title="Create client API key" onClose={() => setKeyOpen(false)}>
        <form onSubmit={handleKeySubmit} className="space-y-4">
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              Key label
            </label>
            <input
              type="text"
              required
              placeholder="e.g. opencode-local, claude-code"
              value={keyName}
              onChange={(e) => setKeyName(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Requests budget (optional)
              </label>
              <input
                type="number"
                min={1}
                placeholder="Unlimited"
                value={keyReqLimit}
                onChange={(e) => setKeyReqLimit(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Tokens budget (optional)
              </label>
              <input
                type="number"
                min={1}
                placeholder="Unlimited"
                value={keyTokLimit}
                onChange={(e) => setKeyTokLimit(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="secondary" size="md" onClick={() => setKeyOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" size="md" disabled={savingKey}>
              {savingKey ? "Generating..." : "Generate key"}
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  );
}
