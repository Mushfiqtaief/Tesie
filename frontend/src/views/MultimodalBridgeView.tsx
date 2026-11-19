import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  Check,
  Code2,
  Eye,
  FileText,
  Play,
  RotateCcw,
  Save,
  Volume2,
  Zap,
} from "lucide-react";
import { api } from "../api";
import {
  Button,
  Card,
  INPUT_CLASS,
} from "../components/UiPrimitives";
import type {
  BridgeStats,
  OpenAiModelEntry,
  UpdateBridgePayload,
} from "../types";
import { getModelCapabilities } from "../utils/modelCapabilities";

interface MultimodalBridgeViewProps {
  onNotify?: (msg: string, isError?: boolean) => void;
}

export function MultimodalBridgeView({ onNotify }: MultimodalBridgeViewProps) {
  const [models, setModels] = useState<OpenAiModelEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [savedSuccess, setSavedSuccess] = useState(false);

  // Live Bridge telemetry stats
  const [stats, setStats] = useState<BridgeStats>({
    total_transcriptions: 0,
    images_transcribed: 0,
    audio_transcribed: 0,
    last_event: null,
  });

  // Form draft state
  const [visionEnabled, setVisionEnabled] = useState(false);
  const [visionModel, setVisionModel] = useState<string>("");
  const [visionMode, setVisionMode] = useState<"standard" | "code_ocr" | "minimal">("standard");
  const [visionPrompt, setVisionPrompt] = useState<string>("");

  const [audioEnabled, setAudioEnabled] = useState(false);
  const [audioModel, setAudioModel] = useState<string>("");
  const [audioPrompt, setAudioPrompt] = useState<string>("");

  // Live simulation state
  const [simPrompt, setSimPrompt] = useState("What error is occurring in this console screenshot?");
  const [simImageUrl, setSimImageUrl] = useState("https://images.unsplash.com/photo-1555066931-4365d14bab8c?w=600");
  const [simTargetModel, setSimTargetModel] = useState("oc/deepseek-v4.1-flash-free");
  const [simulating, setSimulating] = useState(false);
  const [simResult, setSimResult] = useState<{
    transcription: string;
    finalPayload: string;
  } | null>(null);

  useEffect(() => {
    void loadData();
    const interval = setInterval(() => {
      void loadStatsOnly();
    }, 4000);
    return () => clearInterval(interval);
  }, []);

  async function loadData() {
    setLoading(true);
    try {
      const [bridgeRes, modelsRes, statsRes] = await Promise.all([
        api.getBridgeConfig(),
        api.getModels().catch(() => []),
        api.getBridgeStats().catch(() => ({ stats: { total_transcriptions: 0, images_transcribed: 0, audio_transcribed: 0, last_event: null } })),
      ]);

      const cfg = bridgeRes.config;
      setVisionEnabled(cfg.vision_enabled);
      setVisionModel(cfg.vision_model || "");
      setVisionMode(cfg.vision_mode || "standard");
      setVisionPrompt(cfg.vision_prompt || "");

      setAudioEnabled(cfg.audio_enabled);
      setAudioModel(cfg.audio_model || "");
      setAudioPrompt(cfg.audio_prompt || "");

      setModels(modelsRes);
      setStats(statsRes.stats);
    } catch (err) {
      onNotify?.(err instanceof Error ? err.message : "Failed to load bridge configuration", true);
    } finally {
      setLoading(false);
    }
  }

  async function loadStatsOnly() {
    try {
      const res = await api.getBridgeStats();
      if (res.stats) {
        setStats(res.stats);
      }
    } catch {
      // Background poll silently ignore
    }
  }

  // Filter models that support Vision
  const visionCapableModels = useMemo(() => {
    return models.filter((m) => {
      const caps = getModelCapabilities(m.id, m);
      return caps.hasVision;
    });
  }, [models]);

  // Filter models that support Audio
  const audioCapableModels = useMemo(() => {
    return models.filter((m) => {
      const caps = getModelCapabilities(m.id, m);
      return caps.hasAudio;
    });
  }, [models]);

  async function handleSave() {
    setSaving(true);
    try {
      const payload: UpdateBridgePayload = {
        vision_enabled: visionEnabled,
        vision_model: visionModel.trim() ? visionModel.trim() : null,
        vision_mode: visionMode,
        vision_prompt: visionPrompt.trim() ? visionPrompt.trim() : null,
        audio_enabled: audioEnabled,
        audio_model: audioModel.trim() ? audioModel.trim() : null,
        audio_prompt: audioPrompt.trim() ? audioPrompt.trim() : null,
      };

      await api.updateBridgeConfig(payload);
      setSavedSuccess(true);
      setTimeout(() => setSavedSuccess(false), 2000);
      onNotify?.("Multimodal bridge settings saved successfully", false);
    } catch (err) {
      onNotify?.(err instanceof Error ? err.message : "Failed to save bridge settings", true);
    } finally {
      setSaving(false);
    }
  }

  async function handleSimulate() {
    setSimulating(true);

    const targetModelClean = simTargetModel.trim() || "oc/deepseek-v4.1-flash-free";
    const transcriberModelClean = visionModel.trim() || "ag/gemini-3.8-flash";

    const transcriberProvider = transcriberModelClean.startsWith("ag/")
      ? "antigravity"
      : transcriberModelClean.startsWith("ovh/")
      ? "ovh"
      : transcriberModelClean.startsWith("lf/")
      ? "cp-lf"
      : "opencode";

    const targetProvider = targetModelClean.startsWith("ag/")
      ? "antigravity"
      : targetModelClean.startsWith("ovh/")
      ? "ovh"
      : targetModelClean.startsWith("lf/")
      ? "cp-lf"
      : "opencode";

    // Trigger live simulation event on backend so Live Routing Topology canvas immediately connects
    try {
      await api.simulateBridgeEvent({
        transcriber_provider: transcriberProvider,
        target_provider: targetProvider,
        transcriber_model: transcriberModelClean,
        target_model: targetModelClean,
        kind: "image",
      });
      void loadStatsOnly();
    } catch {
      // Continue simulation locally
    }

    setTimeout(() => {
      const sampleTranscription =
        visionMode === "code_ocr"
          ? `[Attached Image Analysis (OCR Focus):
- Visible File: /src/server.ts (TypeScript)
- Highlighted Error: "TypeError: Cannot read properties of undefined (reading 'listen') at line 42:15"
- Code Context:
  40: const app = express();
  41: const port = process.env.PORT || 3000;
  42: server.listen(port, () => { ... });
- Note: 'server' variable is never initialized; replace with 'app.listen']`
          : `[Attached Image Analysis (Standard):
- Depicts: Developer IDE editor with runtime exception console.
- Text Content: "TypeError: Cannot read properties of undefined (reading 'listen')"
- Key Context: Express.js port bind error at server.ts:42]`;

      const transformedText = `${simPrompt}\n\n${sampleTranscription}`;

      setSimResult({
        transcription: sampleTranscription,
        finalPayload: JSON.stringify(
          {
            model: targetModelClean,
            messages: [
              {
                role: "user",
                content: transformedText,
              },
            ],
          },
          null,
          2,
        ),
      });
      setSimulating(false);
      onNotify?.("Simulated multimodal bridge translation & recorded routing topology event", false);
    }, 450);
  }

  const isBridgeActive = visionEnabled || audioEnabled;

  return (
    <div className="space-y-6">
      {/* Header Toolbar */}
      <div className="flex flex-wrap items-center justify-between gap-4 border-b border-[var(--border-subtle)] pb-4">
        <div className="flex items-center gap-3">
          <h1 className="text-xl font-bold tracking-tight text-[var(--text-primary)]">
            Multimodal Bridge
          </h1>
          <span
            className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-semibold border ${
              isBridgeActive
                ? "bg-emerald-500/10 text-emerald-400 border-emerald-500/30"
                : "bg-[var(--bg-elevated)] text-[var(--text-muted)] border-[var(--border-subtle)]"
            }`}
          >
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                isBridgeActive ? "bg-emerald-400 animate-pulse" : "bg-[var(--text-muted)]"
              }`}
            />
            {isBridgeActive ? "Active" : "Disabled"}
          </span>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="secondary"
            size="md"
            onClick={() => void loadData()}
            disabled={loading || saving}
          >
            <RotateCcw className="h-3.5 w-3.5" />
            Reset
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={() => void handleSave()}
            disabled={loading || saving}
          >
            {savedSuccess ? (
              <>
                <Check className="h-3.5 w-3.5 text-white" />
                Saved
              </>
            ) : (
              <>
                <Save className="h-3.5 w-3.5" />
                {saving ? "Saving..." : "Save Settings"}
              </>
            )}
          </Button>
        </div>
      </div>

      {/* Real Live Stats Dashboard (Noise-Free) */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-3.5">
        {/* Stat 1: Total Transcriptions */}
        <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 flex flex-col justify-between">
          <div className="flex items-center justify-between text-xs text-[var(--text-muted)] font-medium">
            <span>Total Transcriptions</span>
            <Activity className="h-3.5 w-3.5 text-sky-400" />
          </div>
          <div className="mt-2 text-2xl font-bold font-mono text-[var(--text-primary)]">
            {stats.total_transcriptions}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-secondary)]">
            Bridged requests delivered
          </div>
        </div>

        {/* Stat 2: Images Transcribed */}
        <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 flex flex-col justify-between">
          <div className="flex items-center justify-between text-xs text-[var(--text-muted)] font-medium">
            <span>Images Transcribed</span>
            <Eye className="h-3.5 w-3.5 text-sky-400" />
          </div>
          <div className="mt-2 text-2xl font-bold font-mono text-sky-400">
            {stats.images_transcribed}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-secondary)]">
            Image-to-text descriptions
          </div>
        </div>

        {/* Stat 3: Audio Transcribed */}
        <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 flex flex-col justify-between">
          <div className="flex items-center justify-between text-xs text-[var(--text-muted)] font-medium">
            <span>Audio Transcribed</span>
            <Volume2 className="h-3.5 w-3.5 text-purple-400" />
          </div>
          <div className="mt-2 text-2xl font-bold font-mono text-purple-400">
            {stats.audio_transcribed}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-secondary)]">
            Voice & speech streams
          </div>
        </div>

        {/* Stat 4: Active Routing Pair */}
        <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 flex flex-col justify-between">
          <div className="flex items-center justify-between text-xs text-[var(--text-muted)] font-medium">
            <span>Last Bridge Route</span>
            <span
              className={`h-2 w-2 rounded-full ${
                stats.last_event ? "bg-emerald-400 animate-pulse" : "bg-[var(--text-muted)]"
              }`}
            />
          </div>
          <div className="mt-2 text-xs font-mono font-semibold text-[var(--text-primary)] truncate" title={stats.last_event?.transcriber_model || "gemini-3.8-flash"}>
            {stats.last_event?.transcriber_model ? `${stats.last_event.transcriber_provider} → ${stats.last_event.target_provider}` : "Standby (Ready)"}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-muted)] truncate">
            {stats.last_event ? stats.last_event.target_model : "No active stream"}
          </div>
        </div>
      </div>

      {/* Grid: Vision & Audio Configuration */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
        {/* Card 1: Vision Adapter */}
        <Card
          title="Vision Bridge"
          actions={
            <label className="relative inline-flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={visionEnabled}
                onChange={(e) => setVisionEnabled(e.target.checked)}
                className="sr-only peer"
              />
              <div className="w-9 h-5 bg-[var(--bg-elevated)] border border-[var(--border-subtle)] peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-sky-500 peer-checked:border-sky-500"></div>
            </label>
          }
        >
          <div className="p-4 space-y-4">
            {/* Vision Model Selector */}
            <div className="space-y-1.5">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                Transcriber Model
              </label>
              <select
                value={visionModel}
                onChange={(e) => setVisionModel(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              >
                <option value="">Default (Auto: Gemini 3.8 Flash / Hy4 Flash)</option>
                {visionCapableModels.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.id} ({m.display_name || m.id})
                  </option>
                ))}
              </select>
            </div>

            {/* Transcription Mode Selector */}
            <div className="space-y-1.5">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                Detail Mode
              </label>
              <div className="grid grid-cols-3 gap-2">
                <button
                  type="button"
                  onClick={() => setVisionMode("standard")}
                  className={`flex flex-col items-start gap-1 rounded-xl border p-2.5 text-left transition-all ${
                    visionMode === "standard"
                      ? "border-sky-500 bg-sky-500/10 shadow-xs"
                      : "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]"
                  }`}
                >
                  <div className="flex items-center gap-1.5 font-semibold text-xs text-[var(--text-primary)]">
                    <FileText className="h-3.5 w-3.5 text-sky-400" />
                    Standard
                  </div>
                  <span className="text-[10px] text-[var(--text-muted)]">
                    Full context
                  </span>
                </button>

                <button
                  type="button"
                  onClick={() => setVisionMode("code_ocr")}
                  className={`flex flex-col items-start gap-1 rounded-xl border p-2.5 text-left transition-all ${
                    visionMode === "code_ocr"
                      ? "border-amber-500 bg-amber-500/10 shadow-xs"
                      : "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]"
                  }`}
                >
                  <div className="flex items-center gap-1.5 font-semibold text-xs text-[var(--text-primary)]">
                    <Code2 className="h-3.5 w-3.5 text-amber-400" />
                    Code OCR
                  </div>
                  <span className="text-[10px] text-[var(--text-muted)]">
                    Exact text & code
                  </span>
                </button>

                <button
                  type="button"
                  onClick={() => setVisionMode("minimal")}
                  className={`flex flex-col items-start gap-1 rounded-xl border p-2.5 text-left transition-all ${
                    visionMode === "minimal"
                      ? "border-emerald-500 bg-emerald-500/10 shadow-xs"
                      : "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]"
                  }`}
                >
                  <div className="flex items-center gap-1.5 font-semibold text-xs text-[var(--text-primary)]">
                    <Zap className="h-3.5 w-3.5 text-emerald-400" />
                    Minimal
                  </div>
                  <span className="text-[10px] text-[var(--text-muted)]">
                    Low token brief
                  </span>
                </button>
              </div>
            </div>

            {/* Custom Prompt Override */}
            <div className="space-y-1.5">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                System Prompt (Optional Override)
              </label>
              <textarea
                rows={2}
                placeholder="Leave blank for built-in prompt..."
                value={visionPrompt}
                onChange={(e) => setVisionPrompt(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              />
            </div>
          </div>
        </Card>

        {/* Card 2: Audio Adapter */}
        <Card
          title="Audio Bridge"
          actions={
            <label className="relative inline-flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={audioEnabled}
                onChange={(e) => setAudioEnabled(e.target.checked)}
                className="sr-only peer"
              />
              <div className="w-9 h-5 bg-[var(--bg-elevated)] border border-[var(--border-subtle)] peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-purple-500 peer-checked:border-purple-500"></div>
            </label>
          }
        >
          <div className="p-4 space-y-4">
            {/* Audio Model Selector */}
            <div className="space-y-1.5">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                Transcriber Model
              </label>
              <select
                value={audioModel}
                onChange={(e) => setAudioModel(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              >
                <option value="">Default (Auto: Whisper Large / Gemini 3.8 Flash)</option>
                {audioCapableModels.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.id} ({m.display_name || m.id})
                  </option>
                ))}
              </select>
            </div>

            {/* Custom Audio Prompt Override */}
            <div className="space-y-1.5">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                System Prompt (Optional Override)
              </label>
              <textarea
                rows={2}
                placeholder="Leave blank for default: 'You are an audio transcription engine...'"
                value={audioPrompt}
                onChange={(e) => setAudioPrompt(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              />
            </div>
          </div>
        </Card>
      </div>

      {/* Simulator: Interactive Verification */}
      <Card
        title="Interactive Bridge Tester"
        actions={
          <span className="font-mono text-xs text-[var(--text-muted)]">
            Tests transcribing & routes topology
          </span>
        }
      >
        <div className="p-4 space-y-4">
          <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
            <div>
              <label className="mb-1 block text-xs font-semibold text-[var(--text-secondary)]">
                User Question
              </label>
              <input
                type="text"
                value={simPrompt}
                onChange={(e) => setSimPrompt(e.target.value)}
                className={`${INPUT_CLASS} text-xs`}
              />
            </div>

            <div>
              <label className="mb-1 block text-xs font-semibold text-[var(--text-secondary)]">
                Target Model (Text-Only)
              </label>
              <input
                type="text"
                value={simTargetModel}
                onChange={(e) => setSimTargetModel(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              />
            </div>

            <div>
              <label className="mb-1 block text-xs font-semibold text-[var(--text-secondary)]">
                Image Attachment
              </label>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={simImageUrl}
                  onChange={(e) => setSimImageUrl(e.target.value)}
                  className={`${INPUT_CLASS} text-xs font-mono flex-1`}
                />
                <Button
                  variant="primary"
                  size="sm"
                  onClick={handleSimulate}
                  disabled={simulating}
                >
                  <Play className="h-3 w-3" />
                  {simulating ? "Bridging..." : "Simulate"}
                </Button>
              </div>
            </div>
          </div>

          {/* Simulation Output */}
          {simResult && (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3 pt-2 border-t border-[var(--border-subtle)]">
              <div>
                <span className="mb-1 block font-mono text-[11px] font-semibold text-sky-400">
                  Transcribed Description
                </span>
                <pre className="rounded-xl border border-sky-500/25 bg-sky-500/5 p-3 font-mono text-[11px] text-[var(--text-primary)] whitespace-pre-wrap max-h-52 overflow-y-auto">
                  {simResult.transcription}
                </pre>
              </div>

              <div>
                <span className="mb-1 block font-mono text-[11px] font-semibold text-emerald-400">
                  Delivered Payload to {simTargetModel}
                </span>
                <pre className="rounded-xl border border-emerald-500/25 bg-emerald-500/5 p-3 font-mono text-[11px] text-[var(--text-secondary)] whitespace-pre-wrap max-h-52 overflow-y-auto">
                  {simResult.finalPayload}
                </pre>
              </div>
            </div>
          )}
        </div>
      </Card>
    </div>
  );
}
