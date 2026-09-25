import { useState } from "react";
import "./App.css";
import RecordingTab from "./RecordingTab";
import InferenceTab from "./InferenceTab";
import SearchTab from "./SearchTab";
import SecretsTab from "./SecretsTab";

const TABS = [
  { id: "recording", label: "녹음", view: <RecordingTab /> },
  { id: "inference", label: "추론", view: <InferenceTab /> },
  { id: "search", label: "검색", view: <SearchTab /> },
  { id: "secrets", label: "자격 증명", view: <SecretsTab /> },
];

function App() {
  const [active, setActive] = useState("recording");
  return (
    <main style={{ padding: 16, fontFamily: "system-ui, sans-serif" }}>
      <nav style={{ display: "flex", gap: 8, marginBottom: 16 }}>
        {TABS.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActive(tab.id)}
            style={{ fontWeight: active === tab.id ? 700 : 400 }}
          >
            {tab.label}
          </button>
        ))}
      </nav>
      {TABS.find((tab) => tab.id === active)?.view}
    </main>
  );
}

export default App;
