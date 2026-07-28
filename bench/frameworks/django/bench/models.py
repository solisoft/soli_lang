from django.db import models


class Post(models.Model):
    """The 50-row read dataset. The table is created and seeded by the shared
    harness, so Django does not manage its schema."""
    title = models.TextField()
    views = models.IntegerField()

    class Meta:
        db_table = "posts"
        managed = False


class Wpost(models.Model):
    """Isolated 800,000-row table for the write workloads."""
    title = models.TextField()
    views = models.IntegerField()

    class Meta:
        db_table = "wposts"
        managed = False
