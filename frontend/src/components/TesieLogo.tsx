import { useId } from "react";

export function TesieLogo({
  size = 36,
  className = "",
}: {
  size?: number;
  className?: string;
}) {
  const gradId = useId();

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
      className={`shrink-0 ${className}`}
      aria-hidden="true"
    >
      <defs>
        <linearGradient
          id={gradId}
          x1="2"
          y1="2"
          x2="30"
          y2="30"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0%" stopColor="#0EA5E9" />
          <stop offset="100%" stopColor="#0284C7" />
        </linearGradient>
      </defs>
      <rect
        x="1.5"
        y="1.5"
        width="29"
        height="29"
        rx="7.5"
        fill={`url(#${gradId})`}
        stroke="#38BDF8"
        strokeOpacity="0.35"
        strokeWidth="1"
      />
      <rect x="7.8" y="8.5" width="16.4" height="4" rx="2" fill="#FFFFFF" />
      <rect x="14" y="9" width="4" height="15.5" rx="2" fill="#FFFFFF" />
      <circle cx="9.6" cy="19.6" r="2.15" fill="#FFFFFF" />
      <circle cx="22.4" cy="19.6" r="2.15" fill="#FFFFFF" />
    </svg>
  );
}
