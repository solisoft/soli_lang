Build the HTTP API described in SPEC.md, in the project in the current directory. The project is a {stack} application, freshly created with `{setup}`.

{notes}

Requirements:

- Write an executable `run.sh` at the project root. It must start the server in the foreground, listening on 127.0.0.1 at the port given by the `PORT` environment variable, and do any preparation the app needs on every start (migrations, for example). Dependencies you install now stay installed.
- Follow SPEC.md exactly: paths, status codes, JSON shapes and validation rules. It will be checked by an automated HTTP test suite you cannot see.
- Data must survive a restart of the server.
- You may start the server and test it yourself (a free port is in `PORT`). Stop every server you started before you finish.
- Work autonomously: nobody will answer questions. Stop when the API is complete and you have checked it.
