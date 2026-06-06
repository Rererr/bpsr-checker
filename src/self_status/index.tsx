/* @refresh reload */
import { render } from "solid-js/web";
import { Show } from "solid-js";
import { SelfStatusOverlay } from "./SelfStatusOverlay";
import { crossWindowFlag } from "../lib/crossWindowFlag";

const root = document.getElementById("root");
if (!root) throw new Error("Root element not found");

// 表示/非表示は main の設定(localStorage)に追従して出し分ける。
// （切替時に main が白紙化するのはこのモニタ固有の WebView2 不具合のため、
//  main 側でトグル後にリロードして復帰させている。SettingsPanel 参照）
render(() => {
  const active = crossWindowFlag("showSelfStatusOverlay");
  return (
    <Show when={active()}>
      <SelfStatusOverlay />
    </Show>
  );
}, root);
