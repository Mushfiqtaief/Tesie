import { useEffect, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  MessageSquare,
  Send,
  SlidersHorizontal,
  Square,
  Trash2,
} from "lucide-react";
import { runPlaygroundChat } from "../api";
import { Button, Card, EmptyState, INPUT_CLASS } from "../components/UiPrimitives";
import type { ApiKeyView, OpenAiModelEntry } from "../types";
import { formatCompactNumber, formatMs, formatTps } from "../utils/format";

interface ChatMessageItem {
  id: string;
  role: "user" | "assistant";
  content: string;
  reasoning?: string;
  proxyUsed?: string | null;
  egressIp?: string | null;
  resolvedModel?: string | null;
  comboUsed?: string | null;
  ttftMs?: number | null;
  latencyMs?: number;
  tokensPerSec?: number | null;
  promptTokens?: number;
  cachedTokens?: number;
  completionTokens?: number;
  totalTokens?: number;
  error?: string;
}

export function PlaygroundView({
  models,
  keys = [],
  initialModel,
  onCompleted,
}: {
  models: OpenAiModelEntry[];
  keys?: ApiKeyView[];
  initialModel: string;
  onCompleted: () => void;
}) {
  const [model, setModel] = useState(initialModel || "oc/deepseek-v4.1-flash-free");
  const [stream, setStream] = useState(true);
  const [apiKey, setApiKey] = useState("");
  const [systemPrompt, setSystemPrompt] = useState(
    "You are a helpful, concise engineering assistant.",
  );
  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<ChatMessageItem[]>([]);
  const [running, setRunning] = useState(false);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [expandedReasoning, setExpandedReasoning] = useState<Record<string, boolean>>({});
  const abortRef = useRef<AbortController | null>(null);
  const scrollEndRef = useRef<HTMLDivElement | null>(null);

  const defaultEnabledKey = keys.find((k) => k.enabled && k.raw_key)?.raw_key ?? "";
  const anyEnabledKeyExists = keys.some((k) => k.enabled);

  useEffect(() => {
    if (initialModel) {
      setModel(initialModel);
    }
  }, [initialModel]);

  useEffect(() => {
    if (!apiKey && defaultEnabledKey) {
      setApiKey(defaultEnabledKey);
    }
  }, [apiKey, defaultEnabledKey]);

  useEffect(() => {
    scrollEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [messages.length]);

  async function handleSend(e?: React.FormEvent) {
    e?.preventDefault();
    const prompt = input.trim();
    if (!prompt || running) return;

    const userMsg: ChatMessageItem = {
      id: `u-${Date.now()}`,
      role: "user",
      content: prompt,
    };
    const asstId = `a-${Date.now() + 1}`;
    const placeholder: ChatMessageItem = {
      id: asstId,
      role: "assistant",
      content: "",
    };

    const historyForApi: Array<{ role: "system" | "user" | "assistant"; content: string }> = [];
    if (systemPrompt.trim()) {
      historyForApi.push({ role: "system", content: systemPrompt.trim() });
    }
    for (const m of messages) {
      if (!m.error && m.content) {
        historyForApi.push({ role: m.role, content: m.content });
      }
    }
    historyForApi.push({ role: "user", content: prompt });

    setMessages((prev) => [...prev, userMsg, placeholder]);
    setInput("");
    setRunning(true);

    const ctrl = new AbortController();
    abortRef.current = ctrl;

    try {
      const result = await runPlaygroundChat({
        model: model.trim(),
        messages: historyForApi,
        stream,
        apiKey: apiKey.trim() || defaultEnabledKey || undefined,
        signal: ctrl.signal,
        onProgress: (p) => {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === asstId
                ? {
                    ...m,
                    content: p.content,
                    reasoning: p.reasoning,
                    proxyUsed: p.proxyUsed,
                    egressIp: p.egressIp,
                    resolvedModel: p.resolvedModel,
                    comboUsed: p.comboUsed,
                    ttftMs: p.ttftMs,
                    latencyMs: p.elapsedMs,
                    tokensPerSec: p.liveTps,
                    completionTokens: p.approxTokens,
                  }
                : m,
            ),
          );
        },
      });

      setMessages((prev) =>
        prev.map((m) =>
          m.id === asstId
            ? {
                ...m,
                content: result.content,
                reasoning: result.reasoning,
                proxyUsed: result.proxyUsed,
                egressIp: result.egressIp,
                resolvedModel: result.resolvedModel,
                comboUsed: result.comboUsed,
                ttftMs: result.ttftMs,
                latencyMs: result.latencyMs,
                tokensPerSec: result.tokensPerSec,
                promptTokens: result.promptTokens,
                cachedTokens: result.cachedTokens,
                completionTokens: result.completionTokens,
                totalTokens: result.totalTokens,
              }
            : m,
        ),
      );
      onCompleted();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Request failed";
      setMessages((prev) =>
        prev.map((m) =>
          m.id === asstId
            ? {
                ...m,
                error: ctrl.signal.aborted ? "Aborted by user" : msg,
              }
            : m,
        ),
      );
      onCompleted();
    } finally {
      abortRef.current = null;
      setRunning(false);
    }
  }

  const formatModelSpec = (m: OpenAiModelEntry) => {
    if (m.id.includes("/") || m.owned_by === "combo") return m.id;
    if (m.owned_by === "antigravity") return `ag/${m.id}`;
    return `oc/${m.id}`;
  };

  const quickModels = models.slice(0, 5).map(formatModelSpec);

  return (
    <div className="space-y-4">
      <Card noPadding className="overflow-hidden">
        {/* Compact Playground Toolbar */}
        <div className="space-y-2.5 border-b border-[var(--border-subtle)] px-4 py-3">
          <div className="flex flex-wrap items-center justify-between gap-2.5">
            <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
              <div className="w-full sm:w-64">
                <input
                  list="tesie-models-list"
                  aria-label="Model (catalog or custom spec)"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  className={`${INPUT_CLASS} py-1 font-mono`}
                />
                <datalist id="tesie-models-list">
                  {models.map((m) => {
                    const val = formatModelSpec(m);
                    return <option key={m.id} value={val} />;
                  })}
                </datalist>
              </div>

              <label className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2.5 py-1 text-xs font-medium text-[var(--text-secondary)]">
                <input
                  type="checkbox"
                  checked={stream}
                  onChange={(e) => setStream(e.target.checked)}
                  className="h-3.5 w-3.5 rounded accent-[var(--accent)]"
                />
                <span>SSE stream</span>
              </label>

              <Button
                variant="ghost"
                size="sm"
                onClick={() => setOptionsOpen((o) => !o)}
                title="Configure client API key and system prompt"
              >
                <SlidersHorizontal className="h-3.5 w-3.5" />
                Options
              </Button>
            </div>

            <Button
              variant="ghost"
              size="sm"
              onClick={() => setMessages([])}
              disabled={running || messages.length === 0}
            >
              <Trash2 className="h-3.5 w-3.5" />
              Clear
            </Button>
          </div>

          {quickModels.length > 0 && (
            <div className="flex flex-wrap items-center gap-1">
              {quickModels.map((qm) => (
                <button
                  key={qm}
                  type="button"
                  onClick={() => setModel(qm)}
                  className={`rounded px-2 py-0.5 font-mono text-[11px] transition-colors ${
                    model === qm
                      ? "bg-[var(--accent-soft)] font-medium text-[var(--accent)]"
                      : "text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  {qm}
                </button>
              ))}
            </div>
          )}

          {optionsOpen && (
            <div className="grid grid-cols-1 gap-2.5 border-t border-[var(--border-subtle)] pt-2.5 md:grid-cols-12">
              <div className="md:col-span-4">
                <label className="mb-1 block text-[11px] font-medium text-[var(--text-muted)]">
                  Client API key ({anyEnabledKeyExists ? "auto-attached" : "optional"})
                </label>
                <input
                  type="password"
                  placeholder={
                    anyEnabledKeyExists
                      ? "Auto-using active client key"
                      : "Optional sk-tesie-..."
                  }
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  className={`${INPUT_CLASS} py-1 font-mono`}
                />
              </div>
              <div className="md:col-span-8">
                <label className="mb-1 block text-[11px] font-medium text-[var(--text-muted)]">
                  System prompt
                </label>
                <input
                  type="text"
                  placeholder="System prompt (optional)"
                  value={systemPrompt}
                  onChange={(e) => setSystemPrompt(e.target.value)}
                  className={`${INPUT_CLASS} py-1`}
                />
              </div>
            </div>
          )}
        </div>

        {/* Transcript */}
        <div className="max-h-[480px] min-h-[260px] space-y-3 overflow-y-auto p-4">
          {messages.length === 0 ? (
            <EmptyState
              icon={<MessageSquare className="h-5 w-5" />}
              title="Ready to stream"
              description="Send a prompt below to measure TTFT, generation speed, cache hits, and exit IP routing."
            />
          ) : (
            messages.map((m) => (
              <div
                key={m.id}
                className={`rounded-xl border px-3.5 py-2.5 text-xs ${
                  m.role === "user"
                    ? "ml-10 border-[color-mix(in_srgb,var(--accent)_30%,transparent)] bg-[var(--accent-soft)]/55"
                    : "mr-4 border-[var(--border-subtle)] bg-[var(--bg-elevated)]/65"
                }`}
              >
                <div className="mb-1.5 flex flex-wrap items-center justify-between gap-2">
                  <span className="font-mono text-[10px] font-semibold uppercase tracking-wide text-[var(--text-muted)]">
                    {m.role}
                  </span>

                  {m.role === "assistant" && (
                    <div className="flex flex-wrap items-center gap-2 font-mono text-[11px] text-[var(--text-muted)] tabular-nums">
                      {m.comboUsed && (
                        <span className="text-[var(--accent)]">
                          {m.comboUsed}
                          {m.resolvedModel ? ` → ${m.resolvedModel}` : ""}
                        </span>
                      )}
                      {m.proxyUsed && (
                        <span>
                          {m.proxyUsed}
                          {m.egressIp ? ` (${m.egressIp})` : ""}
                        </span>
                      )}
                      {m.tokensPerSec !== undefined && m.tokensPerSec !== null && (
                        <span className="font-semibold text-[var(--speed)]">
                          {formatTps(m.tokensPerSec)}
                        </span>
                      )}
                      {m.ttftMs !== undefined && m.ttftMs !== null && (
                        <span>TTFT {formatMs(m.ttftMs)}</span>
                      )}
                      {m.promptTokens !== undefined && (
                        <span>
                          <span className="text-[var(--token-in)]">
                            {formatCompactNumber(m.promptTokens)}↑
                          </span>
                          {m.cachedTokens && m.cachedTokens > 0 ? (
                            <>
                              {" · "}
                              <span className="text-[var(--token-cache)]">
                                {formatCompactNumber(m.cachedTokens)}c
                              </span>
                            </>
                          ) : null}
                          {m.completionTokens && m.completionTokens > 0 ? (
                            <>
                              {" · "}
                              <span className="text-[var(--token-out)]">
                                {formatCompactNumber(m.completionTokens)}↓
                              </span>
                            </>
                          ) : null}
                        </span>
                      )}
                      {m.latencyMs !== undefined && <span>{formatMs(m.latencyMs)}</span>}
                    </div>
                  )}
                </div>

                {m.reasoning && (
                  <div className="mb-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-2.5 py-2">
                    <button
                      type="button"
                      onClick={() =>
                        setExpandedReasoning((prev) => ({ ...prev, [m.id]: !prev[m.id] }))
                      }
                      className="flex items-center gap-1 text-[11px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                    >
                      {expandedReasoning[m.id] ? (
                        <ChevronDown className="h-3.5 w-3.5" />
                      ) : (
                        <ChevronRight className="h-3.5 w-3.5" />
                      )}
                      Reasoning trace ({m.reasoning.length} chars)
                    </button>
                    {expandedReasoning[m.id] && (
                      <pre className="mt-2 whitespace-pre-wrap font-mono text-[11px] text-[var(--text-muted)]">
                        {m.reasoning}
                      </pre>
                    )}
                  </div>
                )}

                {m.error ? (
                  <div className="rounded-lg bg-[var(--danger-soft)] p-2.5 font-mono text-xs text-[var(--danger)]">
                    {m.error}
                  </div>
                ) : (
                  <div className="whitespace-pre-wrap text-[13px] leading-relaxed text-[var(--text-primary)]">
                    {m.content || (running ? "…" : "")}
                  </div>
                )}
              </div>
            ))
          )}
          <div ref={scrollEndRef} />
        </div>

        {/* Prompt Composer */}
        <form
          onSubmit={handleSend}
          className="flex gap-2 border-t border-[var(--border-subtle)] bg-[var(--bg-elevated)]/35 px-4 py-3"
        >
          <input
            type="text"
            placeholder={`Message ${model}...`}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            className={`${INPUT_CLASS} flex-1`}
          />
          {running ? (
            <Button
              type="button"
              variant="danger"
              size="md"
              onClick={() => abortRef.current?.abort()}
            >
              <Square className="h-3.5 w-3.5" />
              Stop
            </Button>
          ) : (
            <Button type="submit" variant="primary" size="md" disabled={!input.trim()}>
              <Send className="h-3.5 w-3.5" />
              Send
            </Button>
          )}
        </form>
      </Card>
    </div>
  );
}
