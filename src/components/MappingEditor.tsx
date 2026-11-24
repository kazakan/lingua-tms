// 엑셀 열 매핑 편집기: 시트, 머리글 행, 열마다 "무시 / 토큰명 / 언어"를 고른다.
// 가져오기 화면과 이력 화면(엑셀 두 개 비교)에서 같이 쓴다.
import type { ColumnMapping, Locale, SheetPreview } from "../api";

const KEY_HEADERS = ["key", "keys", "token", "token key", "id", "string id", "stringid", "name", "토큰", "토큰명", "키"];
const PREVIEW_ROWS = 8;

/** 0 → A, 26 → AA */
export function columnName(col: number): string {
  let s = "";
  let n = col;
  for (;;) {
    s = String.fromCharCode(65 + (n % 26)) + s;
    if (n < 26) return s;
    n = Math.floor(n / 26) - 1;
  }
}

/** 머리글 글자로 열 역할을 추측한다. 언어 코드·이름이 같으면 그 언어, 토큰명처럼 보이면 토큰명 열. */
export function suggestMapping(sheet: SheetPreview, headerRow: number, locales: Locale[]): ColumnMapping {
  const header = sheet.head[headerRow] ?? [];
  const locale_cols: { col: number; locale: string }[] = [];
  const used = new Set<string>();
  let key_col = -1;
  header.forEach((h, col) => {
    const t = h.trim().toLowerCase();
    if (!t) return;
    const loc = locales.find(
      (l) => !used.has(l.code) && (l.code.toLowerCase() === t || l.name.toLowerCase() === t || t.includes(l.code.toLowerCase())),
    );
    if (loc) {
      locale_cols.push({ col, locale: loc.code });
      used.add(loc.code);
    } else if (key_col < 0 && KEY_HEADERS.includes(t)) {
      key_col = col;
    }
  });
  if (key_col < 0) {
    const free = header.findIndex((h, col) => h.trim() !== "" && !locale_cols.some((c) => c.col === col));
    key_col = free >= 0 ? free : 0;
  }
  return { sheet: sheet.name, header_row: headerRow, key_col, locale_cols };
}

type Props = {
  sheets: SheetPreview[];
  locales: Locale[];
  mapping: ColumnMapping;
  onChange: (m: ColumnMapping) => void;
};

export default function MappingEditor({ sheets, locales, mapping, onChange }: Props) {
  const sheet = sheets.find((s) => s.name === mapping.sheet) ?? sheets[0];
  if (!sheet) return <div className="muted">시트가 없습니다</div>;
  const cols = Math.max(sheet.cols, 1);
  const roleOf = (col: number) =>
    col === mapping.key_col ? "__key" : (mapping.locale_cols.find((c) => c.col === col)?.locale ?? "");

  const setRole = (col: number, role: string) => {
    let { key_col, locale_cols } = mapping;
    locale_cols = locale_cols.filter((c) => c.col !== col && c.locale !== role);
    if (role === "__key") key_col = col;
    else {
      if (key_col === col) key_col = -1;
      if (role) locale_cols = [...locale_cols, { col, locale: role }].sort((a, b) => a.col - b.col);
    }
    onChange({ ...mapping, key_col, locale_cols });
  };

  const rows = sheet.head.slice(mapping.header_row, mapping.header_row + PREVIEW_ROWS);

  return (
    <div>
      <div className="row">
        <label>
          시트
          <select
            value={sheet.name}
            onChange={(e) => {
              const s = sheets.find((x) => x.name === e.target.value);
              if (s) onChange(suggestMapping(s, 0, locales));
            }}
          >
            {sheets.map((s) => (
              <option key={s.name} value={s.name}>
                {s.name} ({s.rows}행)
              </option>
            ))}
          </select>
        </label>
        <label>
          머리글 행
          <input
            type="number"
            min={1}
            max={sheet.head.length}
            value={mapping.header_row + 1}
            style={{ width: 64 }}
            onChange={(e) => {
              const r = Math.max(0, Math.min(sheet.head.length - 1, Number(e.target.value) - 1));
              onChange(suggestMapping(sheet, r, locales));
            }}
          />
        </label>
        <span className="muted">열마다 토큰명 또는 언어를 고르세요. 머리글을 보고 자동으로 골라 둡니다.</span>
      </div>
      <div className="table-scroll">
        <table className="table">
          <thead>
            <tr>
              <th />
              {Array.from({ length: cols }, (_, col) => (
                <th key={col}>
                  <div>{columnName(col)}</div>
                  <select value={roleOf(col)} onChange={(e) => setRole(col, e.target.value)}>
                    <option value="">무시</option>
                    <option value="__key">토큰명</option>
                    {locales.map((l) => (
                      <option key={l.code} value={l.code}>
                        {l.name} ({l.code})
                      </option>
                    ))}
                  </select>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((r, i) => (
              <tr key={i} className={i === 0 ? "active" : ""}>
                <td className="muted">{mapping.header_row + i + 1}</td>
                {Array.from({ length: cols }, (_, col) => (
                  <td key={col} className="pre">
                    {r[col] ?? ""}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {mapping.key_col < 0 && <div className="notice">토큰명 열을 골라야 합니다</div>}
    </div>
  );
}

/** 매핑이 쓸 수 있는 상태인지 */
export function mappingReady(m: ColumnMapping | null): m is ColumnMapping {
  return !!m && m.key_col >= 0 && m.locale_cols.length > 0;
}
