document.addEventListener('DOMContentLoaded', function() {
    document.querySelectorAll('.clickable-row').forEach(function(row) {
        row.addEventListener('click', function() {
            var href = this.getAttribute('data-href');
            if (href) window.location.href = href;
        });
    });
});