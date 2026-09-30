# Toolchains for containerised trials. soli, solidb and claude are not installed here:
# bench.py mounts the host's binaries read-only, so every stack runs the same agent build.
# Debian trixie: glibc 2.41, and the soli/solidb release binaries need 2.39.
FROM ruby:3.4-trixie

RUN apt-get update \
 && apt-get install -y --no-install-recommends git sqlite3 libsqlite3-dev build-essential \
      ca-certificates curl procps libyaml-dev \
 && rm -rf /var/lib/apt/lists/*

# Node LTS from the official image, for Next.js (and anything else that wants npm).
COPY --from=node:24-trixie-slim /usr/local/bin/node /usr/local/bin/node
COPY --from=node:24-trixie-slim /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx

# rails new needs the gem; the app's own bundle goes to $BUNDLE_PATH inside the trial.
RUN gem install rails --no-document

# Trials run as the host user (docker --user / podman keep-id): make the gem dirs readable and git happy.
RUN git config --system safe.directory '*' \
 && git config --system user.name agent-bench \
 && git config --system user.email bench@localhost \
 && chmod -R a+rX /usr/local/bundle
