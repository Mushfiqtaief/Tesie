import { useEffect, useMemo, useState } from "react";
import {
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

  // Form draft state
  const [visionEnabled, setVisionEnabled] = useState(false);
  const [visionModel, setVisionModel] = useState<string>("");
  const [visionMode, setVisionMode] = useState<"standard" | "code_ocr" | "minimal">("standard");
  const [visionPrompt, setVisionPrompt] = useState<string>("");

  const [audioEnabled, setAudioEnabled] = useState(false);
  const [audioModel, setAudioModel] = useState<string>("");
  const [audioPrompt, setAudioPrompt] = useState<string>("");

  // Live simulation state
  const [simPrompt, setSimPrompt] = useState("What error is happening in this screenshot?");
  const [simImageUrl, setSimImageUrl] = useState("https://images.unsplash.com/photo-1555066931-4365d14bab8c?w=600");
  const [simTargetModel, setSimTargetModel] = useState("deepseek-chat");
  const [simulating, setSimulating] = useState(false);
  const [simResult, setSimResult] = useState<{
    transcription: string;
    finalPayload: string;
  } | null>(null);

  useEffect(() => {
    void loadData();
  }, []);

  async function loadData() {
    setLoading(true);
    try {
      const [bridgeRes, modelsRes] = await Promise.all([
        api.getBridgeConfig(),
        api.getModels().catch(() => []),
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
    } catch (err) {
      onNotify?.(err instanceof Error ? err.message : "Failed to load bridge configuration", true);
    } finally {
      setLoading(false);
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

  function handleSimulate() {
    setSimulating(true);
    setTimeout(() => {
      const sampleTranscription = visionMode === "code_ocr"
        ? `[Attached Image Analysis (Multimodal Bridge - OCR Focus):
- Image Type: IDE Code Editor / Terminal Screenshot
- Visible File: /src/server.ts (TypeScript)
- Highlighted Error: "TypeError: Cannot read properties of undefined (reading 'listen') at line 42:15"
- Code Context:
  40: const app = express();
  41: const port = process.env.PORT || 3000;
  42: server.listen(port, () => { ... });
- Note: 'server' variable is never initialized; 'app.listen' should be used instead.]`
        : `[Attached Image Analysis (Multimodal Bridge - Standard):
- Image Type: Screenshot of web development IDE showing a compilation failure.
- Key Elements: Terminal console with red error text, code window on the left with line numbers 35-50.
- Text Content: "TypeError: Cannot read properties of undefined (reading 'listen')".]`;

      const transformedText = `${simPrompt}\n\n${sampleTranscription}`;

      setSimResult({
        transcription: sampleTranscription,
        finalPayload: JSON.stringify(
          {
            model: simTargetModel,
            messages: [
              {
                role: "user",
                content: transformedText,
              },
            ],
          },
          null,
          2
        ),
      });
      setSimulating(false);
    }, 450);
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-wrap items-center justify-between gap-4 border-b border-[var(--border-subtle)] pb-4">
        <div>
          <div className="flex items-center gap-2.5">
            <h1 className="text-xl font-bold tracking-tight text-[var(--text-primary)]">
              Multimodal Bridge
            </h1>
            <span
              className={`inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-xs font-semibold border ${
                visionEnabled || audioEnabled
                  ? "bg-emerald-500/10 text-emerald-400 border-emerald-500/30"
                  : "bg-[var(--bg-elevated)] text-[var(--text-muted)] border-[var(--border-subtle)]"
              }`}
            >
              <span
                className={`h-1.5 w-1.5 rounded-full ${
                  visionEnabled || audioEnabled ? "bg-emerald-400 animate-pulse" : "bg-[var(--text-muted)]"
                }`}
              />
              {visionEnabled || audioEnabled ? "Active" : "Inactive"}
            </span>
          </div>
          <p className="mt-1 text-xs text-[var(--text-secondary)]">
            Automatically transcribe image and audio inputs via high-speed vision models before routing to text-only models (e.g. DeepSeek Chat, Llama 3, Qwen Coder).
          </p>
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

      {/* Grid: Vision Adapter & Audio Adapter */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
        {/* Card 1: Vision Bridge */}
        <Card
          title="Vision Bridge (Image-to-Text Fallback)"
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
            <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-3 text-xs text-[var(--text-secondary)] space-y-1">
              <div className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
                <Eye className="h-4 w-4 text-sky-400" />
                How Vision Bridge Operates
              </div>
              <p>
                When a downstream client sends an image to a text-only model, Tesie intercepts the image, calls your selected Vision Adapter model to generate a rich textual description, and passes the description directly to the text-only model.
              </p>
            </div>

            {/* Vision Model Selector */}
            <div className="space-y-1.5">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                Vision Transcriber Model
              </label>
              <select
                value={visionModel}
                onChange={(e) => setVisionModel(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              >
                <option value="">Default (Auto-select fast vision model: Gemini 3.8 Flash / Hy4 Flash)</option>
                {visionCapableModels.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.id} ({m.display_name || m.id})
                  </option>
                ))}
              </select>
              <p className="text-[11px] text-[var(--text-muted)]">
                Only models with verified Vision capability are shown.
              </p>
            </div>

            {/* Transcription Mode Selector */}
            <div className="space-y-1.5">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                Transcription Detail Mode
              </label>
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
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
                  <span className="text-[10px] text-[var(--text-muted)] leading-tight">
                    Comprehensive visual, structural, and text context.
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
                    Code & OCR
                  </div>
                  <span className="text-[10px] text-[var(--text-muted)] leading-tight">
                    Prioritizes exact code snippets, logs, errors, and line numbers.
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
                  <span className="text-[10px] text-[var(--text-muted)] leading-tight">
                    1-2 concise sentences to minimize token footprint.
                  </span>
                </button>
              </div>
            </div>

            {/* Custom Prompt Override */}
            <div className="space-y-1.5 pt-1">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                Custom Vision System Prompt (Optional)
              </label>
              <textarea
                rows={3}
                placeholder="Leave blank to use the built-in optimized prompt for the selected mode..."
                value={visionPrompt}
                onChange={(e) => setVisionPrompt(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              />
            </div>
          </div>
        </Card>

        {/* Card 2: Audio Bridge */}
        <Card
          title="Audio Bridge (Speech-to-Text Fallback)"
          actions={
            <label className="relative inline-flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={audioEnabled}
                onChange={(e) => setAudioEnabled(e.target.checked)}
                className="sr-only peer"
              />
              <div className="w-9 h-5 bg-[var(--bg-elevated)] border border-[var(--border-subtle)] peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-emerald-500 peer-checked:border-emerald-500"></div>
            </label>
          }
        >
          <div className="p-4 space-y-4">
            <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-3 text-xs text-[var(--text-secondary)] space-y-1">
              <div className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
                <Volume2 className="h-4 w-4 text-emerald-400" />
                How Audio Bridge Operates
              </div>
              <p>
                When an audio payload is submitted to a model without audio capabilities, the audio file is automatically transcribed verbatim into text and supplied to your target model.
              </p>
            </div>

            {/* Audio Model Selector */}
            <div className="space-y-1.5">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                Audio Transcriber Model
              </label>
              <select
                value={audioModel}
                onChange={(e) => setAudioModel(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              >
                <option value="">Default (Auto-select Gemini 3.8 Flash / Mimo Omni)</option>
                {audioCapableModels.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.id} ({m.display_name || m.id})
                  </option>
                ))}
              </select>
              <p className="text-[11px] text-[var(--text-muted)]">
                Only models with verified Audio input capability are shown.
              </p>
            </div>

            {/* Custom Audio Prompt Override */}
            <div className="space-y-1.5 pt-1">
              <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                Custom Audio System Prompt (Optional)
              </label>
              <textarea
                rows={3}
                placeholder="Leave blank to use default: 'You are an audio transcription engine. Transcribe this audio verbatim...'"
                value={audioPrompt}
                onChange={(e) => setAudioPrompt(e.target.value)}
                className={`${INPUT_CLASS} text-xs font-mono`}
              />
            </div>
          </div>
        </Card>
      </div>

      {/* Card 3: Interactive Live Bridge Simulator */}
      <Card
        title="Interactive Bridge Simulator"
        actions={
          <span className="font-mono text-xs text-[var(--text-muted)]">
            Simulate image routing to text-only models
          </span>
        }
      >
        <div className="p-4 space-y-4">
          <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
            <div>
              <label className="mb-1 block text-xs font-semibold text-[var(--text-secondary)]">
                User Question / Prompt
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
                placeholder="e.g. deepseek-chat, llama-3"
                className={`${INPUT_CLASS} text-xs font-mono`}
              />
            </div>

            <div>
              <label className="mb-1 block text-xs font-semibold text-[var(--text-secondary)]">
                Image Attachment URL
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
                  1. Generated Multimodal Bridge Description
                </span>
                <pre className="rounded-xl border border-sky-500/25 bg-sky-500/5 p-3 font-mono text-[11px] text-[var(--text-primary)] whitespace-pre-wrap max-h-56 overflow-y-auto">
                  {simResult.transcription}
                </pre>
              </div>

              <div>
                <span className="mb-1 block font-mono text-[11px] font-semibold text-emerald-400">
                  2. Transformed Payload Delivered to {simTargetModel}
                </span>
                <pre className="rounded-xl border border-emerald-500/25 bg-emerald-500/5 p-3 font-mono text-[11px] text-[var(--text-secondary)] whitespace-pre-wrap max-h-56 overflow-y-auto">
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
