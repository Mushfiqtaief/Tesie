import { useState } from "react";
import { Layers } from "lucide-react";

export function ProviderIcon({
  src,
  name,
  color = "#E56A4A",
  size = 36,
  isCombo = false,
}: {
  src?: string | null;
  name: string;
  color?: string;
  size?: number;
  isCombo?: boolean;
}) {
  const [failed, setFailed] = useState(false);

  if (isCombo) {
    return (
      <div
        className="flex shrink-0 items-center justify-center rounded-xl border border-[var(--border-subtle)] bg-[var(--accent-soft)] text-[var(--accent)]"
        style={{ width: size, height: size }}
      >
        <Layers className="h-4 w-4" />
      </div>
    );
  }

  return (
    <div
      className="flex shrink-0 items-center justify-center overflow-hidden rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]"
      style={{
        width: size,
        height: size,
        backgroundColor: `${color}14`,
      }}
    >
      {src && !failed ? (
        <img
          src={src}
          alt={name}
          width={Math.round(size * 0.72)}
          height={Math.round(size * 0.72)}
          onError={() => setFailed(true)}
          className="object-contain rounded-md"
          style={{
            width: Math.round(size * 0.72),
            height: Math.round(size * 0.72),
          }}
        />
      ) : (
        <span
          className="font-mono text-xs font-bold uppercase"
          style={{ color }}
        >
          {name.slice(0, 2)}
        </span>
      )}
    </div>
  );
}
