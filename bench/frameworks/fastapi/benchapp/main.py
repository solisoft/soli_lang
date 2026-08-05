"""FastAPI + SQLAlchemy 2.0 (asyncpg) + Jinja2 — the same seven matched
workloads as the Soli, Rails, Express, AdonisJS, Laravel and Django apps.

Two things about this app are choices rather than defaults, and both are stated
on the results page rather than hidden:

* **The published rows return a `Response` directly.** FastAPI's headline
  feature is that `return rows` runs the value through `jsonable_encoder` (and
  a `response_model` when declared) before serialising it. That is real
  framework work no other stack in this comparison does, so the matched rows use
  `JSONResponse`/`HTMLResponse` — a documented FastAPI idiom ("Return a Response
  Directly"). `/json-encoded` and `/db-encoded` serve the default path instead
  and are published as labelled reference rows, because the gap between them is
  a fact about FastAPI worth knowing.

* **The DB rows project without instantiating models** — `select(Post.id,
  Post.title, Post.views)` through an `AsyncSession`, the SQLAlchemy analogue of
  Rails' `pluck`, Soli's `pluck`, Sequelize's `raw: true`, Eloquent's
  `toBase()` and Django's `.values()`. `/db-hydrated` is the reference form that
  does build 50 mapped objects.
"""

import random
from pathlib import Path

import jinja2
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, Response
from fastapi.templating import Jinja2Templates
from sqlalchemy import delete, select, update

from .models import Post, Session, Wpost

app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)

# autoescape matches Django's and Soli's `h()`. keep_trailing_newline is the
# one non-default: Jinja2 strips a single trailing newline from a template, which
# would make this page one byte shorter than Django's off the same template file.
# With it the two render byte-identical output.
templates = Jinja2Templates(
    env=jinja2.Environment(
        loader=jinja2.FileSystemLoader(str(Path(__file__).parent / "templates")),
        autoescape=True,
        keep_trailing_newline=True,
        auto_reload=False,
    )
)

WPOOL = 800_000

# Projection without hydrating mapped objects — compiled once, not per request.
POSTS = select(Post.id, Post.title, Post.views)


def rows():
    """50 in-memory rows, identical to the other stacks."""
    return [{"id": i, "title": f"Post title {i}", "views": i * 7} for i in range(1, 51)]


async def db_rows():
    async with Session() as session:
        result = await session.execute(POSTS)
        return [dict(row) for row in result.mappings()]


@app.get("/json")
async def json_only():
    return JSONResponse(rows())


@app.get("/json-encoded")
async def json_encoded():
    """Reference: FastAPI's default return path, through `jsonable_encoder`."""
    return rows()


@app.get("/template")
async def template_only(request: Request):
    return templates.TemplateResponse(
        request, "posts/list.html", {"title": "Posts", "items": rows()}
    )


@app.get("/db")
async def db_json():
    return JSONResponse(await db_rows())


@app.get("/db-encoded")
async def db_encoded():
    """Reference: the same read through FastAPI's default return path."""
    return await db_rows()


@app.get("/db-template")
async def db_template(request: Request):
    return templates.TemplateResponse(
        request, "posts/list.html", {"title": "Posts", "items": await db_rows()}
    )


@app.get("/db-hydrated")
async def db_hydrated():
    """Reference: the form that does instantiate 50 mapped objects."""
    async with Session() as session:
        posts = (await session.execute(select(Post))).scalars()
        return JSONResponse(
            [{"id": p.id, "title": p.title, "views": p.views} for p in posts]
        )


# ---- Writes: one operation per request, against `wposts` (800,000 rows) ----
# The key is drawn from the same 1..800000 range as every other stack, so each
# request addresses one row by primary key.


@app.post("/w")
async def w_create():
    async with Session() as session:
        session.add(Wpost(title="Post title 0", views=7))
        await session.commit()
    return Response(status_code=201)


@app.patch("/w")
async def w_update():
    async with Session() as session:
        # synchronize_session=False: the session is fresh and holds no identity
        # map, so there is nothing to synchronise — the default would issue an
        # extra SELECT that Django's `.filter().update()` does not.
        await session.execute(
            update(Wpost)
            .where(Wpost.id == random.randint(1, WPOOL))
            .values(views=42)
            .execution_options(synchronize_session=False)
        )
        await session.commit()
    return Response(status_code=200)


@app.delete("/w")
async def w_delete():
    async with Session() as session:
        await session.execute(
            delete(Wpost)
            .where(Wpost.id == random.randint(1, WPOOL))
            .execution_options(synchronize_session=False)
        )
        await session.commit()
    return Response(status_code=200)
