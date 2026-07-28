import random

from django.http import HttpResponse, JsonResponse
from django.shortcuts import render
from django.views.decorators.csrf import csrf_exempt
from django.views.decorators.http import require_http_methods

from .models import Post, Wpost

WPOOL = 800_000

# json.dumps defaults to ', ' / ': ' separators, which would make Django's
# payload 299 bytes larger than every other stack's for the same 50 rows.
# Compact separators keep the responses byte-identical.
COMPACT = {"separators": (",", ":")}


def _rows():
    """50 in-memory rows, identical to the other stacks."""
    return [{"id": i, "title": f"Post title {i}", "views": i * 7} for i in range(1, 51)]


def _db_rows():
    """Projection without instantiating models — the ORM analogue of Rails'
    pluck, Soli's pluck, Sequelize's raw:true and Eloquent's toBase()."""
    return list(Post.objects.values("id", "title", "views"))


def json_only(request):
    return JsonResponse(_rows(), safe=False, json_dumps_params=COMPACT)


def template_only(request):
    return render(request, "posts/list.html", {"title": "Posts", "items": _rows()})


def db_json(request):
    return JsonResponse(_db_rows(), safe=False, json_dumps_params=COMPACT)


def db_template(request):
    return render(request, "posts/list.html", {"title": "Posts", "items": _db_rows()})


def db_hydrated(request):
    """Reference: the form that does instantiate 50 model objects."""
    rows = [{"id": p.id, "title": p.title, "views": p.views}
            for p in Post.objects.only("id", "title", "views")]
    return JsonResponse(rows, safe=False, json_dumps_params=COMPACT)


@csrf_exempt
@require_http_methods(["POST", "PATCH", "DELETE"])
def write(request):
    if request.method == "POST":
        Wpost.objects.create(title="Post title 0", views=7)
        return HttpResponse(status=201)
    if request.method == "PATCH":
        Wpost.objects.filter(id=random.randint(1, WPOOL)).update(views=42)
        return HttpResponse(status=200)
    Wpost.objects.filter(id=random.randint(1, WPOOL)).delete()
    return HttpResponse(status=200)
