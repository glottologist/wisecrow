# Deploying Wisecrow on calypso

The Wisecrow web app is a Dioxus fullstack server that terminates TLS itself and
serves an authenticated, multi-user study UI backed by PostgreSQL. This runbook
covers a production deployment to the `calypso` host.

## Architecture

- `wisecrow-web` — the fullstack server (axum + rustls); serves HTTPS on 8443.
- PostgreSQL 15 — corpus and per-user data.
- Both run as containers via `docker-compose.deploy.yml`. The `ansible/`
  playbooks sync the repo to the host, render the runtime `.env`, build the
  image, and bring the stack up.

There is no reverse proxy: the application owns TLS termination and certificate
loading.

### Shared host

calypso also runs the trading stack, whose Caddy already binds `:80` and `:443`.
Wisecrow therefore serves on `:8443` (free on this host) and never uses `:80`/
`:443`. Because those ports are taken, the certificate is obtained over the
DNS-01 challenge rather than HTTP-01/`--standalone` (step 1). The `install.yml`
play also refuses to start if anything other than wisecrow's own container is
already listening on the web port.

## Prerequisites

- Docker and the Compose plugin on calypso.
- A public DNS hostname for wisecrow, plus a free deSEC zone and token to hold
  the `_acme-challenge` TXT and a one-off CNAME in the IONOS panel pointing at
  it (for automated DNS-01 issuance), or your own certificate to drop in
  (step 1).
- Ansible on the control machine (to run the `ansible/` playbooks), or run
  `docker compose` directly on the host.

## 1. TLS certificates

By default the `wisecrow` Ansible role obtains and renews a Let's Encrypt
certificate over the **DNS-01** challenge — no inbound ports are touched, which
is required here because the trading stack's Caddy already owns `:80`/`:443`
(see [Shared host](#shared-host)).

IONOS has no usable DNS API on this account, so the challenge is **CNAME-
delegated** to a free [deSEC](https://desec.io) zone: certbot runs `--manual`
and a hook writes the `_acme-challenge` TXT to deSEC over its REST API, while
IONOS keeps serving the rest of the zone. One-time setup:

1. Create a free deSEC account and a delegation domain (e.g.
   `wisecrow-acme.dedyn.io`), then a token under **Token Management**.
2. In the IONOS DNS panel add a single static CNAME:
   `_acme-challenge.wisecrow` → `_acme-challenge.wisecrow-acme.dedyn.io`.
3. Set in `inventory/group_vars/all.yml`:
   - `wisecrow_tls_domain` — the public FQDN, e.g. `wisecrow.glottologist.co.uk`.
   - `wisecrow_tls_email` — Let's Encrypt registration / expiry contact.
   - `wisecrow_desec_domain` — the deSEC zone, e.g. `wisecrow-acme.dedyn.io`.
4. Put the deSEC token in the vault as `wisecrow_desec_token` (step 2).

On `install.yml` the role installs the challenge hook, issues the cert, copies
`fullchain.pem` + `privkey.pem` into the cert directory with ownership for uid
10001, and installs a renewal deploy-hook so future renewals reload the running
container automatically.

> After writing the TXT the hook waits `wisecrow_desec_propagation` seconds
> (default 60) before ACME validates.

**Bringing your own certificate instead.** Set `wisecrow_tls_obtain_cert:
false` and place `fullchain.pem` + `privkey.pem` in the cert directory
(`wisecrow_cert_dir`, default `./certs` next to the compose file) yourself. The
container runs as uid 10001, so the files must be readable by it:

```sh
sudo chown 10001:root certs/privkey.pem && sudo chmod 0400 certs/privkey.pem
sudo chown 10001:root certs/fullchain.pem && sudo chmod 0644 certs/fullchain.pem
```

The files are owned by uid 10001 rather than made group-readable because the
image creates its user with `useradd --system --user-group`, which draws the
gid from the system range (999) instead of matching `--uid`.

The server loads certs only at start-up, so restart the container after any
out-of-band renewal (set `wisecrow_tls_renewal_hook: false` to leave the
auto-reload hook out entirely):

```sh
docker compose -f docker-compose.deploy.yml restart wisecrow-web
```

## 2. Secrets

Copy the example, fill it in, and vault-encrypt it:

```sh
cp ansible/vars/secrets.yml.example ansible/vars/secrets.yml
$EDITOR ansible/vars/secrets.yml
ansible-vault encrypt ansible/vars/secrets.yml
```

- Use **distinct** values for `postgres_password` and `become_pw_calypso` (the
  host sudo password). Never reuse one for the other.
- Treat any password that has been committed or shared in plaintext as
  compromised, and **rotate it** — change it in Postgres / on the host and
  update the vault.
- `sync_api_key_secret` is the optional legacy single sync key; prefer per-client
  keys (step 6).
- `unsplash_api_key` enables optional learning-card images. Leave it blank to
  keep image enrichment disabled; audio TTS does not require an API key.
- `wisecrow_desec_token` is the deSEC API token used to write the DNS-01
  challenge record (step 1). Leave blank if `wisecrow_tls_obtain_cert` is false.

## 3. Build and start

With ansible:

```sh
ansible-playbook -i ansible/inventory/hosts.yml ansible/playbooks/preflight.yml
ansible-playbook -i ansible/inventory/hosts.yml ansible/playbooks/install.yml --ask-vault-pass
ansible-playbook -i ansible/inventory/hosts.yml ansible/playbooks/start.yml
```

`install.yml` syncs the repo, renders `.env` (mode 0600), builds the image, and
brings the stack up. The production web bundle includes server-side TTS and
optional Unsplash images without the CLI-only local audio playback dependency.
The play waits for both containers to report healthy and runs a `list-languages`
smoke test.

Or directly on the host:

```sh
POSTGRES_PASSWORD=... docker compose -f docker-compose.deploy.yml up -d --build
```

## 4. Create the first admin

There is no public signup. Bootstrap the first admin account on the host once the
stack is up:

```sh
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  env WISECROW__INIT_PASSWORD='choose-a-strong-password' \
  wisecrow user add --email you@example.com --display-name "You" --admin
```

`WISECROW__INIT_PASSWORD` provisions non-interactively; omit it to be prompted.
Manage further accounts:

```sh
wisecrow user add --email user@example.com --display-name "User"
wisecrow user list
wisecrow user passwd --email user@example.com
wisecrow user disable --email user@example.com   # clears the password, revokes sessions
```

## 5. Populate language data

A freshly installed instance has an empty `translations` table, and every study
flow refuses to start without one: `learn`, `nback` and the web equivalents
report "Not enough vocabulary … Ingest data first". Populating the corpus is
therefore a required step, not an optional one, and it is deliberately manual —
no playbook does it, because which language pairs an instance carries is a
decision rather than a default.

The runtime image ships the full CLI at `/usr/local/bin/wisecrow` alongside the
web server, and the compose service already exports every `WISECROW__DB_*`
variable it needs. Everything below therefore runs through `exec` on the
container that is already up.

### Where the downloads land

`ingest` writes each archive to its working directory and does not delete it
afterwards. The container's `WORKDIR` is `/app/web`, which has no volume behind
it, so an unqualified ingest quietly fills the container's writable layer with
several gigabytes that `docker volume ls` will never show and the next
`--build` will discard. Run ingests with an explicit working directory and
clear it afterwards:

```sh
docker compose -f docker-compose.deploy.yml exec --workdir /tmp wisecrow-web \
  wisecrow ingest -n en -f es
docker compose -f docker-compose.deploy.yml exec wisecrow-web sh -c 'rm -f /tmp/*.tmx*'
```

Check free space first. A pair's archives are held compressed and expanded at
the same time, and the NLLB releases are the heavy ones — the Irish TMX arrives
as 1.3 GB and expands to roughly 5 GB.

### Choose the corpora

Five OPUS collections are available: `open_subtitles`, `cc_aligned`,
`cc_matrix`, `paracrawl` and `nllb`. Omitting `--corpus` requests all five.
Coverage is uneven, and no collection spans every pair; a pair a collection
does not carry returns 404, which is logged once, not retried, and does not
stop the others. For the Celtic languages the gaps are worth knowing in
advance, since they are not the ones intuition suggests:

| Pair | Best value | Notes |
|------|-----------|-------|
| `en`–`cy` | `cc_aligned` (837k pairs, 70 MB) | No CCMatrix release at all. |
| `en`–`ga` | `paracrawl` (3.2M pairs, 320 MB) | No CCAligned release. |
| `en`–`gd` | `cc_matrix` (310k pairs, 19 MB) | Thinnest of the three by far. |

NLLB carries far more for each, but expands to between 1.6 GB and 5 GB per
pair; raise `--max-decompressed-mb` above its 8192 default only if a release
genuinely needs it.

```sh
docker compose -f docker-compose.deploy.yml exec --workdir /tmp wisecrow-web \
  wisecrow ingest -n en -f cy --corpus "open_subtitles cc_aligned"
```

### Import a memory OPUS does not carry

Where no collection covers a pair well, `--file` ingests a TMX from disk — a
published government translation memory, say, or a Tatoeba export. Copy it in,
decompressed, and point at it:

```sh
docker compose -f docker-compose.deploy.yml cp ./cy-en-legislation.tmx wisecrow-web:/tmp/
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  wisecrow ingest --file /tmp/cy-en-legislation.tmx -n en -f cy
```

Any TMX 1.4 file works provided its `<tuv>` elements carry `xml:lang`
attributes matching the codes passed.

### Apply frequencies

Ingestion leaves every new row at a frequency of 1, and the deck query skips
rows at that value, so a corpus is close to unusable until a frequency list has
ranked it. For the languages Hermit Dave covers, one command suffices:

```sh
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  wisecrow frequency --lang es
```

Hermit Dave publishes nothing for Welsh, Irish or Gaelic. Use a Leipzig Corpora
Collection word list instead, which `--file` reads in its native
`rank<TAB>word<TAB>count` layout:

```sh
curl -O https://downloads.wortschatz-leipzig.de/corpora/cym_wikipedia_2021_100K.tar.gz
tar xzf cym_wikipedia_2021_100K.tar.gz
docker compose -f docker-compose.deploy.yml cp cym_wikipedia_2021_100K wisecrow-web:/tmp/
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  wisecrow frequency --lang cy \
  --file /tmp/cym_wikipedia_2021_100K/cym_wikipedia_2021_100K-words.txt
```

Leipzig has Welsh (`cym`) and Irish (`gle`) but no Gaelic corpus; for `gd` a
list derived from a Wikipedia dump is the practical route. A list ranks a
translation from whichever side of the pair its language sits on, so the
ingest direction does not matter.

### Grammar and media (optional)

Grammar rules need an LLM provider configured (step 2); media pre-fetching
avoids a first-session stall while audio is generated.

```sh
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  wisecrow seed-grammar --lang cy --levels A1,A2,B1
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  wisecrow prefetch-media -n en -f cy
```

### Confirm what landed

```sh
docker compose -f docker-compose.deploy.yml exec postgres \
  psql -U wisecrow -d wisecrow -c \
  "SELECT lf.code AS from_lang, lt.code AS to_lang,
          count(*) AS pairs, count(*) FILTER (WHERE frequency > 1) AS ranked
     FROM translations t
     JOIN languages lf ON lf.id = t.from_language_id
     JOIN languages lt ON lt.id = t.to_language_id
    GROUP BY 1, 2 ORDER BY 3 DESC;"
```

The `ranked` column is the one that matters: those are the rows a deck can
draw on.

## 6. Sync-client keys (optional)

If another instance pulls the corpus from this one, provision a per-client key:

```sh
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  wisecrow sync-client add --name laptop   # prints the key once — store it
wisecrow sync-client list
wisecrow sync-client revoke --name laptop
```

The puller sends the key as the `x-api-key` header. Keys are individually
revocable and compared in constant time.

## 7. Verify

```sh
curl -sk https://<host>:8443/ -o /dev/null -w '%{http_code}\n'   # a response over TLS
docker compose -f docker-compose.deploy.yml ps                    # both healthy
```

Log in at `https://<host>:8443/login`.

## Configuration reference (environment)

| Variable | Purpose |
|---|---|
| `WISECROW__DB_*` | Database connection (set by compose from the secrets). |
| `WISECROW__TLS_CERT_PATH` / `WISECROW__TLS_KEY_PATH` | PEM cert/key paths inside the container (`/certs/*`). |
| `IP` / `PORT` | Bind address (default `0.0.0.0:8443`). |
| `WISECROW__LLM_PROVIDER` / `WISECROW__LLM_API_KEY` | LLM provider for gloss / graded-reader / quizzes. |
| `WISECROW__LLM_RATELIMIT_PER_MIN` | Per-user LLM request cap (default 20). |
| `WISECROW__UNSPLASH_API_KEY` | Optional image enrichment key; omit to disable images gracefully. |
| `WISECROW__SYNC_API_KEY` | Legacy single sync key (per-client keys preferred). |
| `RUST_LOG` / `RUST_BACKTRACE` | Logging (backtrace off by default in the image). |

## Updating

```sh
ansible-playbook -i ansible/inventory/hosts.yml ansible/playbooks/update.yml --ask-vault-pass
```

Database migrations apply automatically on start-up.
