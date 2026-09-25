import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const METHODS = ["trigram", "unicode61", "like"];

export default function SearchTab() {
  const [database, setDatabase] = useState("");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Record<string, number[]>>({});
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setError(null);
    const found: Record<string, number[]> = {};
    for (const method of METHODS) {
      try {
        found[method] = await invoke<number[]>("search_notes", {
          database: database.replace("<방식>", method),
          query,
          method,
          limit: 20,
        });
      } catch (caught) {
        setError(String(caught));
        found[method] = [];
      }
    }
    setResults(found);
  };

  return (
    <section>
      <h2>한국어 검색</h2>
      <label style={{ display: "block" }}>
        DB 경로(방식 자리에는 <code>&lt;방식&gt;</code>을 넣는다)
        <input
          value={database}
          placeholder="C:\녹음\검증\corpus-<방식>.db"
          onChange={(event) => setDatabase(event.currentTarget.value)}
          style={{ width: "100%" }}
        />
      </label>
      <label style={{ display: "block", marginTop: 8 }}>
        검색어
        <input value={query} onChange={(event) => setQuery(event.currentTarget.value)} style={{ width: "100%" }} />
      </label>
      <button onClick={run} disabled={database.length === 0 || query.length === 0} style={{ marginTop: 8 }}>
        세 방식으로 검색
      </button>
      {error ? <p style={{ color: "#c00" }}>오류: {error}</p> : null}
      <ul>
        {METHODS.map((method) => (
          <li key={method}>
            {method}: {results[method]?.length ?? 0}건 {results[method]?.slice(0, 5).join(", ")}
          </li>
        ))}
      </ul>
    </section>
  );
}
