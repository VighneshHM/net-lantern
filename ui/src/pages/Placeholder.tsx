export function Placeholder({ title, phase }: { title: string; phase: number }) {
  return (
    <>
      <h1 className="m-0 text-3xl font-bold">{title}</h1>
      <p className="m-0 text-sm text-text-muted">Arrives in phase {phase}</p>
    </>
  );
}
