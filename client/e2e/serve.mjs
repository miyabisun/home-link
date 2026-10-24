// Starts a mock matterjs-server and the built home-link on a fresh ledger for the E2E tests.
// Needs `cargo build` first; HOME_LINK_BIN overrides the binary.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { rmSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { WebSocketServer } from "ws";

const { E2E_DB, PORT, MATTER_PORT } = process.env;
const binary =
  process.env.HOME_LINK_BIN ??
  new URL("../../target/debug/home-link", import.meta.url).pathname;

// A light, a plug and an air conditioner as matterjs-server lists them; On/Off and the
// written attributes follow the commands sent.
const nodes = [
  {
    node_id: 1,
    available: true,
    attributes: {
      "0/40/1": "Tapo",
      "0/40/15": "E2E-LIGHT-1",
      "0/40/3": "Smart Bulb",
      "1/29/0": [{ 0: 269, 1: 1 }],
      "1/6/0": false,
    },
  },
  {
    node_id: 2,
    available: true,
    attributes: {
      "0/40/1": "Tapo",
      "0/40/15": "E2E-PLUG-1",
      "0/40/3": "P110M",
      "1/29/0": [{ 0: 266, 1: 1 }],
      "1/6/0": false,
    },
  },
  {
    node_id: 3,
    available: true,
    attributes: {
      "0/40/1": "Airco",
      "0/40/15": "E2E-AIRCON-1",
      "0/40/3": "Room Air Conditioner",
      "1/29/0": [{ 0: 114, 1: 1 }],
      "1/6/0": true,
      "1/513/28": 3,
      "1/513/17": 2500,
      "1/513/18": 2300,
      "1/514/0": 5,
    },
  },
];

// GSI's address search and Open-Meteo: an address with 見つからない is not found, any
// other is the one GSI found for 東京都新宿区西新宿2-8-1; it is 30.2℃ outside now.
function outside(url, response) {
  response.setHeader("content-type", "application/json");
  if (url.pathname === "/gsi") {
    const missing = (url.searchParams.get("q") ?? "").includes("見つからない");
    response.end(
      missing
        ? "[]"
        : JSON.stringify([
            {
              geometry: { coordinates: [139.691772, 35.689629], type: "Point" },
              type: "Feature",
              properties: {
                addressCode: "",
                title: "東京都新宿区西新宿二丁目８番",
              },
            },
          ]),
    );
  } else {
    const quarter = new Date(Math.floor(Date.now() / 900_000) * 900_000);
    response.end(
      JSON.stringify({
        utc_offset_seconds: 0,
        current: {
          time: quarter.toISOString().slice(0, 16),
          interval: 900,
          temperature_2m: 30.2,
        },
      }),
    );
  }
}

// Plain GETs answer once the ledger is seeded: the tests wait on that.
const http = createServer((request, response) => {
  const url = new URL(request.url ?? "/", "http://localhost");
  if (url.pathname === "/gsi" || url.pathname === "/meteo")
    outside(url, response);
  else response.end("ok");
});
const matter = new WebSocketServer({ server: http });
matter.on("connection", (ws) => {
  ws.send(JSON.stringify({ fabric_id: 1, schema_version: 13 }));
  ws.on("message", (data) => {
    const { message_id, command, args } = JSON.parse(String(data));
    const node = nodes.find((n) => n.node_id === args?.node_id);
    let result = {};
    if (command === "get_nodes") result = nodes;
    else if (command === "read_attribute") {
      result = Object.fromEntries(
        args.attribute_path.map((p) => [p, node?.attributes[p]]),
      );
    } else if (command === "write_attribute" && node) {
      node.attributes[args.attribute_path] = args.value;
      result = [{ Path: {}, Status: 0 }];
    } else if (command === "device_command" && node) {
      if (args.cluster_id === 6)
        node.attributes[`${args.endpoint_id}/6/0`] = args.command_name === "On";
    } else {
      ws.send(
        JSON.stringify({
          message_id,
          error_code: 1,
          details: `mock: ${command}`,
        }),
      );
      return;
    }
    ws.send(JSON.stringify({ message_id, result }));
  });
});

for (const suffix of ["", "-wal", "-shm"])
  rmSync(E2E_DB + suffix, { force: true });
const server = spawn(binary, {
  stdio: "inherit",
  env: {
    ...process.env,
    PORT,
    DATABASE_PATH: E2E_DB,
    MATTER_SERVER_URL: `ws://127.0.0.1:${MATTER_PORT}/ws`,
    GSI_URL: `http://127.0.0.1:${MATTER_PORT}/gsi`,
    OPEN_METEO_URL: `http://127.0.0.1:${MATTER_PORT}/meteo`,
    LOG_LEVEL: "warn",
  },
});
const stop = () => {
  server.kill("SIGTERM");
  http.close();
};
process.on("SIGTERM", stop);
process.on("SIGINT", stop);
server.on("exit", (code) => process.exit(code ?? 0));

// The ledger once home-link has made it: devices are recorded by connecting them, which the app does.
for (let attempt = 0; ; attempt++) {
  try {
    if ((await fetch(`http://127.0.0.1:${PORT}/healthz`)).ok) break;
  } catch {
    if (attempt > 100) throw new Error("home-link did not start");
  }
  await new Promise((resolve) => setTimeout(resolve, 100));
}
const db = new DatabaseSync(E2E_DB);
const insert = db.prepare(
  "INSERT INTO devices (name, vendor, serial_number, qr_payload) VALUES (?, ?, ?, ?)",
);
insert.run("居間の照明", "Tapo", "E2E-LIGHT-1", null);
insert.run("", "Tapo", "E2E-PLUG-1", null);
insert.run("押入れ1", null, null, "MT:Y.K9042C00KA0648G00");
insert.run(
  "寝室の天井の照明（窓側）と書斎の机の上のスタンドライトをまとめて呼ぶための長い名前",
  "Tapo",
  "E2E-GONE",
  null,
);
insert.run("エアコン", "Airco", "E2E-AIRCON-1", null);
db.close();
http.listen(Number(MATTER_PORT), "127.0.0.1");
