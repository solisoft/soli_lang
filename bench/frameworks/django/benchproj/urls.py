from django.urls import path

from bench import views

urlpatterns = [
    path("json", views.json_only),
    path("template", views.template_only),
    path("db", views.db_json),
    path("db-template", views.db_template),
    path("db-hydrated", views.db_hydrated),
    path("w", views.write),
]
