# Readiness, media queue alerts, and response headers

`/livez` is a dependency-free process check. `/readyz` and the deployer's
existing `/healthz` endpoint return 503 when PostgreSQL cannot answer within
two seconds, a required in-process media/composition/archive worker exits, a
media or composition lease remains expired for two minutes, or the oldest due
queued job is over one hour old. When B2 archiving is enabled, archive work
over one hour old also makes the instance unready. A missing/conflicted archive
object is reported but does not by itself remove the whole API from service.

Readiness is not liveness: configure process restart checks against `/livez`
and routing/deploy checks against `/readyz` (or `/healthz`). The response
includes `failed_workers` and `queues`. Queue ages use the oldest *due* item,
so scheduled retries do not trip readiness before their retry time.

The web process records OTLP gauges every 30 seconds, independent of health
probes: `web_server_ready`, `media_worker_failed`, and per-queue
`media_queue_queued_jobs`, `media_queue_active_processes` (leased running
attempts), and `media_queue_oldest_age_seconds`. The existing archive metrics
continue to report pending bytes, upload failures, and verification failures.
Alert on `web_server_ready == 0`, `media_worker_failed > 0`, or queue age
approaching the readiness threshold. Readiness transitions also log an error
and recovery logs an info message; inspect the response before restarting.

The API applies CSP, HSTS, frame, MIME, referrer, and permissions headers to
every response, including error, OAuth, media, and CORS preflight responses.
The OAuth callback uses a single-use script nonce; Scalar's CDN exception is
limited to `/scalar`. The tracked preview Nginx template applies the matching
browser-facing headers to static SPA responses, including `index.html` and
`version.json`. HSTS is only honored by browsers over HTTPS; any separate
production frontend TLS terminator must carry the same static-page policy and
be verified on a deployed normal page and error page. Do not apply the static
SPA CSP to `/api/`, since it would block the callback's nonce script.
