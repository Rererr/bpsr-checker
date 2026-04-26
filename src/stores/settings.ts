import { createSignal } from "solid-js";

const [opacity, setOpacity] = createSignal(0.85);
const [showCrit, setShowCrit] = createSignal(true);
const [showLucky, setShowLucky] = createSignal(true);
const [showHpm, setShowHpm] = createSignal(false);
const [showScore, setShowScore] = createSignal(false);

export {
  opacity, setOpacity,
  showCrit, setShowCrit,
  showLucky, setShowLucky,
  showHpm, setShowHpm,
  showScore, setShowScore,
};
