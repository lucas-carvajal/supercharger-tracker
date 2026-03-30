# Akamai Bot Manager Bypass: Design Doc

## Problem Statement

The scraper currently gets blocked by Akamai Bot Manager after scraping for a while. Two manual workarounds are required today:

1. **VPN location switching**: When blocked, the user must switch to a different VPN exit node and re-run `retry-failed` mode, repeating until all chargers are fetched.
2. **Headed browser (`--show-browser`)**: Running with a visible Chrome window is required for reliable operation; headless mode gets blocked almost immediately.

Neither of these is viable for a server that runs automatically on a schedule.

### Why headless mode gets blocked

Akamai Bot Manager uses five detection layers simultaneously:

- **IP reputation**: Datacenter IPs are heavily flagged; residential IPs are trusted
- **TLS fingerprinting (JA3/JA4)**: The TLS handshake of headless Chrome differs slightly from a real browser
- **JavaScript challenge**: Akamai injects JS that checks `navigator.webdriver`, hardware concurrency, Canvas/WebGL fingerprints, and other signals that differ in automation contexts
- **Behavioral analysis**: Headless sessions have unnaturally consistent timing, no mouse jitter, no scroll events
- **Session monitoring**: The `_abck` and `bm_sz` cookies encode behavioral state; fresh/invalid sessions are flagged

Running with `--show-browser` helps because headed Chrome is more complete in its HTML5 API behavior and because the browser process itself behaves more like a real user session. The VPN switching works because it resets the IP reputation score and forces a new session, giving a fresh slate.

### What needs to change for server deployment

| Problem | Server constraint |
|---|---|
| `--show-browser` requires a display | Servers have no physical monitor |
| VPN switching is manual | No human available to switch VPN |
| Blocking is intermittent | Scraper must self-recover without intervention |

---

## Solution Areas

### A. Virtual Display — Xvfb

**What it is:** Xvfb (X Virtual Framebuffer) is a Linux X11 display server that renders entirely in memory. It exposes a standard display interface that Chrome can use, so headed mode works on a server with no physical monitor.

**How to use it:**

```bash
xvfb-run -a --server-args="-screen 0 1280x800x24 -ac -nolisten tcp -dpi 96 +extension RANDR" \
  ./supercharger-tracker scrape --show-browser
```

The `-a` flag auto-selects a free display number. A display of `1280x800` at 24-bit color is sufficient.

**Why this matters:** This is a near-zero-code-change solution. The existing chromiumoxide code runs unchanged; only the deployment environment changes. Used widely in CI/CD (Playwright's official Docker images include Xvfb).

**Docker setup:**

```dockerfile
RUN apt-get install -y xvfb
CMD ["xvfb-run", "-a", "./supercharger-tracker", "scrape", "--show-browser"]
```

**Limitations:** Xvfb alone does not solve IP blocking. It must be paired with proxy rotation (see below).

---

### B. Residential Proxy Rotation (replaces VPN switching)

**What it is:** Managed pools of residential IP addresses (real ISP subscribers) that rotate per-request or per-session. When Akamai blocks an IP, the proxy service automatically switches to a new one — no manual intervention needed.

**Why residential matters:** Akamai's IP reputation database flags entire datacenter subnets (AWS, GCP, DO, Hetzner). Residential IPs from Comcast, AT&T, etc. are trusted by default.

**Integration with chromiumoxide:** Chrome accepts an HTTP proxy via a launch argument:

```rust
// In loaders.rs, add to stealth_args:
"--proxy-server=http://user:pass@proxy.provider.com:8080"
```

Proxy providers expose a single endpoint; rotation happens server-side. You can request sticky sessions (5–60 minutes) so Akamai session cookies (`_abck`, `bm_sz`) remain valid across batches.

**Provider comparison:**

| Provider | Price | Pool size | Notes |
|---|---|---|---|
| Bright Data | ~$1.77–2.10/GB | 150M+ IPs | Largest pool; 50% off first 3 months |
| Oxylabs | ~$3.49–4/GB | 100M+ IPs | Good documentation |
| SmartProxy / Decodo | ~$2–2.80/GB | 100M+ IPs | City/state targeting |

**Volume estimate for this project:** The scraper fetches ~21k locations but only re-fetches failed ones in retry mode. Bandwidth is dominated by the initial 9.5MB location list plus detail fetches. Monthly proxy cost is likely in the $10–50 range at residential rates.

**Sticky session configuration:** Use a sticky session for each scrape run (one consistent residential IP for the full session), only rotating if Akamai blocks the current IP. This matches the behavior of the current VPN approach but fully automated.

---

### C. Chaser-Oxide (Rust-native Akamai stealth)

**What it is:** An experimental fork of chromiumoxide ([github.com/ccheshirecat/chaser-oxide](https://github.com/ccheshirecat/chaser-oxide)) that adds protocol-level stealth specifically targeting Akamai.

**What it does differently from stock chromiumoxide:**
- Uses `Page.createIsolatedWorld` to run scripts outside the detectable Puppeteer/CDP utility world
- Injects stealth scripts during document creation (before Akamai's JS challenge fires)
- Synchronizes `navigator.platform`, WebGL vendor/renderer, and hardware concurrency to realistic values
- Removes CDP world names that Akamai flags as automation signals

**Integration:** Swap the Cargo.toml dependency:

```toml
# Replace:
chromiumoxide = { version = "0.7", features = ["tokio-runtime"] }
# With:
chaser-oxide = { git = "https://github.com/ccheshirecat/chaser-oxide" }
```

The API surface is identical. The stealth improvements are applied at the protocol layer.

**Caveats:** This is experimental and not widely battle-tested. It may lag behind chromiumoxide's upstream API. Worth evaluating in a branch before committing.

---

### D. Managed Scraping APIs (fully outsourced)

These services receive a URL, handle all Akamai layers internally (TLS fingerprinting, JS challenge, proxy rotation, behavioral simulation), and return the page content or API response.

| Service | Success rate vs Akamai | Pricing | Notes |
|---|---|---|---|
| Zyte API | 97.82% (Proxyway independent test) | Credit-based | Best-tested; formerly Scrapy Cloud |
| Scrapfly | 97–100% (claimed) | Credit-based; free tier 1k credits | Dynamic proxy upgrades mid-request |
| ZenRows | ~97% | $69–299/mo (tiered) | All-in-one; good docs |
| ScraperAPI | 99.99% (claimed) | Per-request | 200M+ proxy pool |

**What changes in the codebase:** The `load_from_browser` function in `src/loaders.rs` (lines 83–127) would be replaced with an HTTP call to the scraping API. The response handling and JSON parsing stay the same.

**Tradeoff:** Simplest to maintain (no browser to manage), highest cost, but you no longer control the fingerprinting or retry logic.

---

### E. Cloud Browser Services

These let you run your existing Playwright/CDP-style automation code remotely on their infrastructure, which already has bot detection bypass baked in.

- **Bright Data Scraping Browser**: $9.50/GB bandwidth + $0.10/hr compute. Connect via WebSocket; your chromiumoxide code targets their remote Chrome rather than a local one.
- **Browserless.io**: $50–200/mo (unit-based, 1 unit = 30s of browser time). Free tier: 1000 units/mo.

**Integration pattern:**

```rust
// Instead of launching a local browser:
Browser::connect("wss://brd-customer-xxxx@brd.superproxy.io:9222").await?
```

Minimal code changes; bot detection is handled by the service.

---

## Recommended Approach

### Phase 1 — Xvfb + Residential Proxy Rotation (short term)

This requires the fewest code changes and can be implemented quickly:

1. **Xvfb**: Wrap the scraper invocation with `xvfb-run` in deployment (systemd unit, Docker CMD, or cron). No Rust code changes.
2. **Proxy rotation**: Add `--proxy-server=...` to the chromiumoxide launch args in `src/loaders.rs`. Load proxy credentials from environment variables (e.g., `PROXY_URL`). Configure sticky sessions with auto-rotation on block.
3. **Block detection**: The scraper already detects HTML responses as blocks (`"API returned HTML (access denied)"`). Wire this to trigger a proxy IP rotation before retrying.

**Estimated cost:** $10–50/mo for residential proxies at this scraping volume.

**Estimated effort:** 1–2 days.

### Phase 2 — Chaser-Oxide Evaluation (medium term)

Test chaser-oxide as a Cargo.toml swap in a separate branch. Run A/B comparison with and without Xvfb to determine whether the stealth improvements eliminate the need for headed mode entirely. This would simplify deployment (no Xvfb needed).

### Phase 3 — Managed API Migration (fallback)

If Akamai continues adapting and Phase 1+2 require constant maintenance, migrate `load_from_browser` to a managed API like Zyte or ZenRows. The fetch layer is well-isolated in `src/loaders.rs`, making this a contained change.

---

## Tradeoff Summary

| Approach | Cost/mo | Code change | Reliability | Infrastructure ownership |
|---|---|---|---|---|
| Xvfb + residential proxies | $10–50 | Minimal | Medium–High | Full |
| Chaser-oxide (drop-in) | $0 (+ proxies) | Minimal | Unknown (experimental) | Full |
| Managed API (ZenRows etc.) | $69–299 | Medium | High (97%+) | None |
| Cloud browser service | $50–200+ | Minimal | Very High | None |

---

## Open Questions

1. What's the target scrape frequency? (Daily, hourly?) This affects proxy bandwidth cost.
2. What server environment is planned? (VPS, Docker, serverless?) This affects Xvfb setup complexity.
3. Is there a budget ceiling for ongoing proxy/API costs?
4. Is the goal to eventually run fully unattended or will occasional manual intervention remain acceptable?
