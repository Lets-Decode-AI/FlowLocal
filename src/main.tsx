import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import "./styles/tokens.css";
import "./styles/base.css";
import "./main.css";
import { currentPlatform } from "./components/ui/Keycap";
import { initTheme } from "./lib/theme";

if (currentPlatform() === "mac") document.documentElement.classList.add("platform-mac");
initTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
