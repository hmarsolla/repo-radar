import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import "./index.css";
import { installGlobalErrorHandlers } from "@/lib/report-error";

installGlobalErrorHandlers();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
