/* @refresh reload */
import { render } from "solid-js/web";
import { SelfStatusOverlay } from "./SelfStatusOverlay";

const root = document.getElementById("root");
if (!root) throw new Error("Root element not found");

render(() => <SelfStatusOverlay />, root);
