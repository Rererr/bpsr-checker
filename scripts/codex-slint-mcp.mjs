import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.dirname(scriptDir);
const port = Number.parseInt(process.env.SLINT_MCP_PORT ?? "8080", 10);
const endpoint = `http://127.0.0.1:${port}/mcp`;

let appReady;

async function isServerReady() {
  try {
    const response = await fetch(endpoint, {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: {
          protocolVersion: "2025-06-18",
          capabilities: {},
          clientInfo: { name: "codex-slint-mcp", version: "0.1.0" },
        },
      }),
    });
    if (!response.ok) {
      return false;
    }
    const body = await response.json();
    return body?.jsonrpc === "2.0" && body?.result?.serverInfo?.name === "slint-mcp-embedded";
  } catch {
    return false;
  }
}

function startElevatedApp() {
  const launcher = path.join(repoRoot, "scripts", "start-slint-mcp.ps1");
  spawn("powershell.exe", [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    launcher,
    "-Port",
    String(port),
    "-LaunchOnly",
  ], {
    cwd: repoRoot,
    stdio: "ignore",
    windowsHide: true,
  });
}

async function ensureAppReady() {
  if (await isServerReady()) {
    return;
  }
  if (!appReady) {
    appReady = (async () => {
      startElevatedApp();
      const deadline = Date.now() + 180_000;
      while (Date.now() < deadline) {
        if (await isServerReady()) {
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 250));
      }
      throw new Error(`Slint MCP did not start at ${endpoint} within 180 seconds`);
    })();
  }
  await appReady;
}

function writeResponse(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

function writeError(id, message) {
  writeResponse({
    jsonrpc: "2.0",
    id: id ?? null,
    error: { code: -32603, message },
  });
}

function writeEventStream(body) {
  for (const event of body.split(/\r?\n\r?\n/)) {
    const data = event
      .split(/\r?\n/)
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice(5).trimStart())
      .join("\n");
    if (data) {
      try {
        writeResponse(JSON.parse(data));
      } catch {
        writeError(null, "Slint MCP returned invalid JSON event data");
      }
    }
  }
}

async function forward(message) {
  await ensureAppReady();
  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      Accept: "application/json, text/event-stream",
      "Content-Type": "application/json",
    },
    body: JSON.stringify(message),
  });
  const body = await response.text();

  if (!response.ok) {
    throw new Error(`Slint MCP returned HTTP ${response.status}: ${body}`);
  }
  if (!body.trim()) {
    return;
  }
  if (response.headers.get("content-type")?.includes("text/event-stream")) {
    writeEventStream(body);
  } else {
    writeResponse(JSON.parse(body));
  }
}

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on("line", (line) => {
  if (!line.trim()) {
    return;
  }
  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    writeError(null, `Invalid JSON-RPC input: ${error.message}`);
    return;
  }
  void forward(message).catch((error) => {
    writeError(message.id, error instanceof Error ? error.message : String(error));
  });
});

input.on("close", () => {
  process.exit(0);
});
