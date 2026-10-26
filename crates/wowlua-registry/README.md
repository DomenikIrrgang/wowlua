# wowlua-registry

The server of the wowlua registry, <https://registry.wowlua.dev>: the
protocol of the spec's [Package manager §10.10](../../spec/toolchain/package-manager.md#1010-registry)
and [Publishing §11.8](../../spec/toolchain/publishing.md#118-wowlua-registry).
`wowlua` is its client: registry dependencies (`"lib-a": "^1.0"`),
`wowlua package --registry`, `wowlua publish` of a library,
`wowlua login registry`, `wowlua yank`, `wowlua owner` and `wowlua search`.

It speaks plain HTTP; HTTPS ends at the proxy in front of it (Traefik on
the droplet). Accounts are GitHub accounts: `/login` signs in with GitHub
and the account page creates and revokes API tokens (stored as their
SHA-256 only), yanks versions and manages owners.

## Running it

```console
$ docker build -f crates/wowlua-registry/Dockerfile -t wowlua-registry .
$ docker run -p 8080:8080 -v wowlua-registry:/data \
    -e WOWLUA_REGISTRY_URL=http://localhost:8080 \
    -e WOWLUA_REGISTRY_GITHUB_CLIENT_ID=… -e WOWLUA_REGISTRY_GITHUB_CLIENT_SECRET=… \
    -e WOWLUA_REGISTRY_ADMINS=DomenikIrrgang \
    wowlua-registry
```

or without Docker, `cargo run -p wowlua-registry` with the same variables
(`WOWLUA_REGISTRY_DATA` names a folder of your own then). To use a local
registry from `wowlua`, set `"registries": { "default": "http://localhost:8080" }`
in `~/.wowlua/config.json` (plain HTTP is accepted for this computer only).

## Environment

| Variable | Default | Meaning |
|----------|---------|---------|
| `WOWLUA_REGISTRY_URL` | required | The public URL, `https://registry.wowlua.dev`: what `config.json` names as `api` and `dl`, and where GitHub returns to (`<url>/login/callback`). Cookies are `Secure` when it is HTTPS. |
| `WOWLUA_REGISTRY_GITHUB_CLIENT_ID` | required | The GitHub OAuth app's client ID |
| `WOWLUA_REGISTRY_GITHUB_CLIENT_SECRET` | required | The GitHub OAuth app's client secret |
| `WOWLUA_REGISTRY_ADMINS` | none | GitHub logins, comma-separated, who assign reserved names and may act as an owner of every package |
| `WOWLUA_REGISTRY_DATA` | `/data` | The data directory |
| `WOWLUA_REGISTRY_ADDR` | `0.0.0.0:8080` | Where it listens |
| `WOWLUA_REGISTRY_MAX_ARCHIVE` | `10485760` (10 MiB) | The largest package archive a publish may upload, in bytes; larger is 413 |
| `WOWLUA_REGISTRY_TRUSTED_PROXIES` | none | Addresses or networks (`172.16.0.0/12`, `10.0.0.5`, `fd00::/8`), comma-separated, of the proxies whose `X-Forwarded-For` names the client. Set it to Traefik's address or its Docker network: without it every request counts against the proxy's address. |
| `WOWLUA_REGISTRY_RATE_PUBLISH` | `30/hour` | Publishes per client address |
| `WOWLUA_REGISTRY_RATE_LOGIN` | `10/minute` | Sign-ins (`/login/github`, `/login/callback`) per address |
| `WOWLUA_REGISTRY_RATE_WRITE` | `120/hour` | Other changes (yank, owners, reserved names, account forms) per address |
| `WOWLUA_REGISTRY_RATE_READ` | `600/minute` | Reads (index, downloads, search, pages) per address |
| `WOWLUA_REGISTRY_GITHUB_URL`, `WOWLUA_REGISTRY_GITHUB_API_URL` | `https://github.com`, `https://api.github.com` | GitHub's addresses (tests point them elsewhere) |

The GitHub OAuth app (GitHub → Settings → Developer settings → OAuth Apps)
has the homepage URL `https://registry.wowlua.dev` and the authorization
callback URL `https://registry.wowlua.dev/login/callback`. The registry asks
for no scopes: it reads the public login only.

A rate is `<count>/<second|minute|hour|day>`, `<count>/<seconds>`, or
`off`. Each is a token bucket per client address and class: up to `count`
at once, refilled evenly over the period. Over it, the answer is 429 with
`Retry-After` (seconds) and the error code `rate-limited`. `/health` is
never limited. The client's address is the connection's, or, when the
connection comes from a trusted proxy, the last `X-Forwarded-For` entry that
is not a trusted proxy (an entry a client wrote itself stays to the left of
the one Traefik adds, so it is never read).

`GET /health` answers `ok`; `wowlua-registry health` asks it (the image's
`HEALTHCHECK`). The server stops cleanly on SIGTERM.

## Data

Everything lives in the data directory, as plain files written through a
temporary file and a rename; one server holds `.lock` and writes one change
at a time, so concurrent requests are safe. Back it up as a folder.

| Path | Content |
|------|---------|
| `index/<name>` | The index file: one JSON line per version (§10.10.1) |
| `archives/<name>/<version>.tar.gz` | The package archives |
| `owners/<name>.json` | `{"users": [GitHub logins]}` |
| `meta/<name>.json` | The newest version's description, keywords and kind, for search |
| `tokens.json` | API tokens: id, SHA-256, login, name, creation time |
| `reserved.json` | The admins' reserved names, over the built-in ones |

Sessions of the sign-in page are kept in memory: a restart signs everyone
out of the page (API tokens keep working).

## Reserved names

Names of well-known WoW libraries (`libstub`, `ace3`, `aceaddon-3-0`,
`libsharedmedia-3-0`, …) and wowlua's own (`wow-api-<flavor>`) are reserved
for their authors: [`data/reserved.json`](data/reserved.json) lists them,
with no authors assigned, so only admins can publish them until an admin
assigns them. Admins extend and assign the list on the account page, or:

```console
$ curl -X PUT -H "Authorization: Bearer $TOKEN" -d '{"users": ["their-login"], "note": "LibFoo"}' \
    https://registry.wowlua.dev/v1/reserved/libfoo
$ curl -X DELETE -H "Authorization: Bearer $TOKEN" https://registry.wowlua.dev/v1/reserved/libfoo
```

`DELETE` releases a name: first come, first served again. `GET /v1/reserved`
lists them.

## Protocol

| Request | |
|---------|---|
| `GET /config.json` | `{"api": "<url>", "dl": "<url>/dl"}` |
| `GET /index/<name>` | The index file, with `ETag` / `If-None-Match` (304) |
| `GET /dl/<name>/<version>.tar.gz` | An archive (yanked ones too) |
| `PUT /v1/packages/<name>/<version>` | Publish: `multipart/form-data` with `manifest` and `archive`; 201 `{"name", "vers", "cksum"}` |
| `DELETE /v1/packages/<name>/<version>` | Yank |
| `PUT /v1/packages/<name>/<version>/unyank` | Undo a yank |
| `GET` / `PUT` / `DELETE /v1/packages/<name>/owners` | List, add, remove owners (`{"users": [...]}`; added logins must exist on GitHub; the last owner stays) |
| `GET /v1/search?q=<words>&page=<n>&per_page=<n>` | Search by name, keywords and description, ranked and paged: `{"packages": [{"name", "vers", "description", "keywords", "kind", "score"}], "total", "page", "per_page"}` |
| `GET` / `PUT` / `DELETE /v1/reserved[/<name>]` | Reserved names (changes: admins) |

Errors are `{"errors": [{"code": "…", "detail": "…"}]}` with 400
(`invalid-name`, `invalid-version`, `invalid-manifest`, `manifest-mismatch`,
`invalid-archive`, `addon`, `dependency-source`, `unknown-user`,
`last-owner`, `invalid-request`), 401 (`unauthorized`), 403 (`not-owner`,
`reserved`, `not-admin`), 404 (`not-found`), 409 (`version-exists`), 413
(`too-large`) and 429 (`rate-limited`).

A publish is checked with the manifest rules of the server's wowlua version
(`wowlua-manifest`), against the URL's name and version and the archive's
`wowlua.json`; the archive must hold regular files only, with safe paths
(the client's rules, `wowlua-pkg`'s `registry` module). The server computes
the checksum (the package's file tree, spec §10.6).

## Why axum

The server runs on [axum](https://crates.io/crates/axum) over tokio and
hyper: the most used and actively maintained HTTP stack in Rust, with
`multipart/form-data` and body limits built in. Its features are cut to
HTTP/1, forms, queries and multipart; the protocol code itself is
synchronous and runs on blocking threads, sharing `wowlua-pkg` and
`wowlua-manifest` with the client. GitHub is called with `ureq`, which the
workspace already uses. The small synchronous alternative, `tiny_http`, has
had no release since 2022.
