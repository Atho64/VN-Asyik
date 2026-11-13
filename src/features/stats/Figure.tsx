/** One label/value pair in a stats figures grid. */
export function Figure({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint?: string;
}) {
  return (
    <div className="figures__item">
      <dt className="figures__label">{label}</dt>
      <dd className="figures__value tnum">{value}</dd>
      {hint && <dd className="figures__hint">{hint}</dd>}
    </div>
  );
}
