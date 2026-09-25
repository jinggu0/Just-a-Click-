import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function SecretsTab() {
  const [value, setValue] = useState("");
  const [saved, setSaved] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setSaved(await invoke<boolean>("notion_token_saved"));
      setError(null);
    } catch (caught) {
      setError(String(caught));
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const save = async () => {
    try {
      await invoke("save_notion_token", { value });
      setValue("");
      await refresh();
    } catch (caught) {
      setError(String(caught));
    }
  };

  const remove = async () => {
    try {
      await invoke("delete_notion_token");
      await refresh();
    } catch (caught) {
      setError(String(caught));
    }
  };

  return (
    <section>
      <h2>자격 증명</h2>
      <p>저장 위치: Windows 자격 증명 관리자 <code>dev.justaclick/notion</code></p>
      <p>현재 상태: {saved === null ? "확인 중" : saved ? "저장됨" : "없음"}</p>
      <label style={{ display: "block" }}>
        Notion 내부 연결 토큰
        <input
          type="password"
          value={value}
          onChange={(event) => setValue(event.currentTarget.value)}
          style={{ width: "100%" }}
        />
      </label>
      <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
        <button onClick={save} disabled={value.length === 0}>
          저장
        </button>
        <button onClick={remove}>삭제</button>
      </div>
      {error ? <p style={{ color: "#c00" }}>오류: {error}</p> : null}
      <p>화면과 기록에는 토큰 값을 표시하지 않는다.</p>
    </section>
  );
}
