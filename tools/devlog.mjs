// Loop 2: prints Luma Player's log from the TV (dev builds post it here; URLs arrive cut to
// their host). Usage: node tools/devlog.mjs [port], default 8081.
import { createServer } from "node:http";

const port = Number(process.argv[2] ?? 8081);

createServer((request, response) => {
  if (request.method !== "POST") {
    response.writeHead(204).end();
    return;
  }
  let body = "";
  request.setEncoding("utf8");
  request.on("data", (chunk) => {
    body += chunk;
    if (body.length > 1_000_000) {
      request.destroy();
    }
  });
  request.on("end", () => {
    for (const line of body.split("\n")) {
      if (line) {
        console.log(line);
      }
    }
    response.writeHead(204).end();
  });
}).listen(port, "0.0.0.0", () => {
  console.log(`Luma Player log server on port ${port}`);
});
