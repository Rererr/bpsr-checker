/* @refresh reload */
import { render } from "solid-js/web";
import { BuffOverlay } from "./BuffOverlay";

const root = document.getElementById("root");
if (!root) throw new Error("Root element not found");

render(() => <BuffOverlay />, root);
