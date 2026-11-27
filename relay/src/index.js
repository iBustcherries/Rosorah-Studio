// Rosorah's report relay: takes crash, disconnect and bug reports from Rosorah Studio and files each
// as an issue in the private reports repository. The GitHub token lives here as a secret, never in
// the app, which is public.
const KINDS = new Set(["crash", "disconnect", "bug"]);
const MAX_BYTES = 200_000;

function reply(value, status = 200) {
  return new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
}
function text(value, max) {
  return typeof value === "string" ? value.slice(0, max) : "";
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method !== "POST" || url.pathname !== "/report") {
      return reply({ error: "Not found" }, 404);
    }
    if (!request.headers.get("X-Rosorah-Version")) {
      return reply({ error: "Not a Rosorah report" }, 400);
    }
    // A few reports a minute per address keeps spam out of the repository.
    const address = request.headers.get("CF-Connecting-IP") || "unknown";
    if (env.PER_ADDRESS) {
      const { success } = await env.PER_ADDRESS.limit({ key: address });
      if (!success) return reply({ error: "too many reports, try again in a minute" }, 429);
    }
    const raw = await request.text();
    if (raw.length > MAX_BYTES) return reply({ error: "report too large" }, 413);
    let report;
    try {
      report = JSON.parse(raw);
    } catch {
      return reply({ error: "not JSON" }, 400);
    }
    const kind = KINDS.has(report.kind) ? report.kind : null;
    const title = text(report.title, 150).replace(/[\r\n]+/g, " ").trim();
    const body = text(report.body, 62_000);
    const version = text(report.version, 32).replace(/[^0-9A-Za-z.\-]/g, "");
    const os = report.os === "windows" || report.os === "linux" ? report.os : null;
    if (!kind || !title || !body) return reply({ error: "missing kind, title or body" }, 400);

    const labels = [kind];
    if (os) labels.push(os);
    const response = await fetch(`https://api.github.com/repos/${env.REPO}/issues`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${env.GITHUB_TOKEN}`,
        accept: "application/vnd.github+json",
        "x-github-api-version": "2022-11-28",
        "user-agent": "rosorah-report-relay",
        "content-type": "application/json",
      },
      body: JSON.stringify({
        title: `[${kind}] ${title}`,
        body: `${body}\n\n---\nRosorah ${version || "?"} · ${os || "unknown system"}`,
        labels,
      }),
    });
    if (!response.ok) {
      return reply({ error: `GitHub turned it down (HTTP ${response.status})` }, 502);
    }
    const issue = await response.json();
    return reply({ number: issue.number });
  },
};
