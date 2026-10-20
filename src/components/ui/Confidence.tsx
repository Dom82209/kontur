export function Confidence({ value }: { value: number }) {
  const level = value >= 0.85 ? "high" : value >= 0.6 ? "mid" : "low";
  const on = Math.max(1, Math.round(value * 5));
  const label = level === "high" ? "уверенно" : level === "mid" ? "проверьте" : "неясно";
  return (
    <span className="confidence" data-level={level} title={`Уверенность распознавания ${Math.round(value * 100)}%`}>
      <span className="confidence__bars" aria-hidden>
        {[0, 1, 2, 3, 4].map((i) => (
          <span key={i} className="confidence__bar" data-on={i < on} style={{ height: 5 + i * 2 }} />
        ))}
      </span>
      <span className="num">{Math.round(value * 100)}%</span>
      <span className="sr-only">, {label}</span>
    </span>
  );
}
