# Laravel Octane on FrankenPHP: the application stays resident between requests,
# where php-fpm rebuilds it every time. Same app, same Eloquent, same Blade —
# only the runtime differs, which is the whole point of the comparison.
FROM dunglas/frankenphp:latest
RUN install-php-extensions pdo_pgsql opcache pcntl
WORKDIR /var/www/html
