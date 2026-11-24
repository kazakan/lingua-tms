// 값 변경 한 칸 표시: 지운 값은 빨간 취소선, 새 값은 초록.
export default function Change({ old, next }: { old: string | null | undefined; next: string | null | undefined }) {
  return (
    <span className="pre">
      {old != null && <del>{old}</del>}
      {old != null && next != null && " → "}
      {next != null ? <ins>{next}</ins> : <span className="muted">(지움)</span>}
    </span>
  );
}
