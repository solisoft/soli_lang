"""SQLAlchemy 2.0 mapped models + the async engine.

The `posts` (50 rows) and `wposts` (800,000 rows) tables are created and seeded
by the shared harness, so nothing here creates schema — these are declarations
over tables that already exist, the analogue of Django's `managed = False`.
"""

import os

from sqlalchemy import Integer, Text
from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Base(DeclarativeBase):
    pass


class Post(Base):
    """The 50-row read dataset."""

    __tablename__ = "posts"

    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    title: Mapped[str] = mapped_column(Text)
    views: Mapped[int] = mapped_column(Integer)


class Wpost(Base):
    """Isolated 800,000-row table for the write workloads."""

    __tablename__ = "wposts"

    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    title: Mapped[str] = mapped_column(Text)
    views: Mapped[int] = mapped_column(Integer)


# Pool of 5 per worker x 16 workers = 80 connections, matching Puma's 16x5,
# Express's `max: 5` per cluster worker and Django's CONN_MAX_AGE. Without a
# pool asyncpg would open a fresh connection per request — worth ~8ms on
# loopback, which measures connection setup rather than the framework.
#
# `max_overflow=0` keeps the cap hard: 5 in flight per worker, no burst above
# it, so the FastAPI column is bounded exactly like the others rather than
# opening as many sockets as the event loop has requests.
#
# POOL_SIZE exists to answer the question the p99 column provokes — whether the
# matched pool is what produces the tail latency. It is a diagnostic, not a
# tuning knob: the published rows are the default 5.
engine = create_async_engine(
    "postgresql+asyncpg://bench:bench@127.0.0.1:5433/bench",
    pool_size=int(os.environ.get("POOL_SIZE", "5")),
    max_overflow=0,
    # Server-side prepared statements are asyncpg's default and its main
    # advantage; nothing else here is tuned.
    echo=False,
)

Session = async_sessionmaker(engine, expire_on_commit=False)
