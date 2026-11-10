import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";


function dismissBootSplash() {
  const splash = document.getElementById("boot-splash");
  if (!splash) return;

  splash.classList.add("is-hidden");

  const remove = () => {
    splash.remove();
  };

  splash.addEventListener("transitionend", remove, { once: true });
  window.setTimeout(remove, 400);
}

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

root.render(
  <React.StrictMode>
    <App onReady={dismissBootSplash} />
  </React.StrictMode>,
);

window.setTimeout(dismissBootSplash, 6000);