import React from "react";
import ReactDOM from "react-dom/client";
import { ErrorBoundary } from "./components/error-boundary";
import App from "./App";
import { installFileDropGuard } from "./lib/file-drop-guard";
import { installSpotlight } from "./lib/spotlight";
import "./index.css";

installSpotlight();
installFileDropGuard();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
);
