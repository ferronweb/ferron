<a href="https://ferron.sh" target="_blank">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="wwwroot/assets/logodark.svg">
    <img alt="Ferron logo" src="wwwroot/assets/logo.svg" width="192">
  </picture>
</a>

# **Ferron** - szybki, nowoczesny serwer WWW z myślą o łatwiejszym debugowaniu

[![Static Badge](https://img.shields.io/badge/Documentation-orange?style=flat-square)](https://ferron.sh/docs)
[![Website](https://img.shields.io/website?url=https%3A%2F%2Fferron.sh&style=flat-square)](https://ferron.sh)
[![Chat](https://img.shields.io/matrix/ferronweb%3Amatrix.org?style=flat-square)](https://matrix.to/#/#ferronweb:matrix.org)
[![X (formerly Twitter) Follow](https://img.shields.io/twitter/follow/ferron_web?style=flat-square)](https://x.com/ferron_web)
[![Docker Pulls](https://img.shields.io/docker/pulls/ferronserver/ferron?style=flat-square)](https://hub.docker.com/r/ferronserver/ferron)
[![GitHub Repo stars](https://img.shields.io/github/stars/ferronweb/ferron?style=flat-square)](https://github.com/ferronweb/ferron)

[English](./README.md) | Polski

## Dlaczego Ferron?

Stworzony dla szybkiej konfiguracji, przewidywalności i niezawodności w środowisku produkcyjnym.

- **Czytelna konfiguracja** - postaw strony internetowe i odwrotne proxy z jasną, czytelną konfiguracją bez ukrytych szczegółów.
- **Automatyczne TLS** - certyfikaty są wystawiane i odświeżane automatycznie. Otrzymujesz jasne sygnały, gdy (nie) działa.
- **Obserwowalność pierwszej klasy** - zobacz dokładnie, co się stało z każdym żądaniem. Ślady dotyczą każdej warstwy i odsyłają się bezpośrednio do odpowiednich dzienników.
- **Przewidywalna wydajność** - szybki i spójny pod obciążeniem, od razu po uruchomieniu. Nie trzeba dostrajać.
- **Bezpieczeństwo pamięci** - cały zakres podatności związanych z bezpieczeństwem pamięci po prostu nie istnieje w Ferronie (jest napisany w [Rust](https://rust-lang.org/)).
- **Niezawodny** - radzi sobie z chaotycznym ruchem rzeczywistości, awariami serwerów upstream i przypadkami brzegowymi protokołów, przewidywalnie.

> [!tip]
> Ferron jest stworzony w myślą o dwóch zasadach: **łatwej konfiguracji** (w ciągu kilku minut) i **łatwym debugowaniu** (gdy coś pójdzie nie tak, szybko znajdziesz przyczynę).

## Przykładowe konfiguracje

### Pliki statyczne

```ferron
example.com {
    root "/var/www/html"

    # Jeśli odkomentujesz, przeglądanie zawartości katalogów zostanie włączone.
    #directory_listing
}
```

### Odwrotny proxy ("reverse proxy")

```ferron
api.example.com {
    proxy http://localhost:8080
}
```

Więcej przykładów można znaleźć w [dokumentacji związanej z konfiguracją (po angielsku)](https://ferron.sh/docs/configuration/fundamentals/syntax).

## Instalacja Ferrona (wersja gotowa do użycia)

Najprostszym sposobem na zaczynanie z Ferronem jest użycie skryptu instalacyjnego dla systemu Linux:

```sh
sudo bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

Zobacz pełne instrukcje w [dokumentacji związanej z instalacją na Linuxie (po angielsku)](https://ferron.sh/docs/installation/linux/installer).

## Kompilacja ze źródła

```sh
git clone https://github.com/ferronweb/ferron -b develop-3.x
cd ferron
git submodule update --init --recursive
cargo build --workspace
```

Uruchom serwer:

```sh
cargo run -p ferron -- run -c ferron.conf
cargo run -p ferron -- run -c ferron.conf --verbose  # z dziennikami debugowania
```

Inne polecenia:

```sh
cargo run -p ferron -- validate -c ferron.conf   # sprawdź konfigurację bez uruchamiania
cargo run -p ferron -- adapt -c ferron.conf      # wyświetl konfigurację jako JSON
cargo run -p ferron -- daemon -c ferron.conf --pid-file /var/run/ferron.pid  # demon Unix
```

Uruchom testy i sprawdź:

```sh
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Spakuj Ferron w celu dystrybucji (wymaga `just`):

```sh
just package # Archiwum (.zip dla Windows, .tar.gz dla Unix)
just package-deb # Pakiet Debian
just package-rpm # Pakiet RPM
just package-windows # Instalator Windows
just installer # Instalator Linux
```

Zoptymalizowane programy kompilowane krzyżowo (zobacz [README dla plików budowania (po angielsku)](./cross-build/README.md); dostępne tylko na Linux):

```sh
just cross-build
```

## Konfiguracja

Pełna referencja do dyrektyw jest dostępna w [docs/configuration/ (po angielsku)](https://ferron.sh/docs/configuration/fundamentals/syntax).

## Wkład

Opinie, zgłoszenia błędów i testy są mile widziane. Gdy zgłaszasz błąd, podaj konfigurację, wyjście z `--verbose` oraz kroki do odtworzenia. Zobacz [CONTRIBUTING.md](./CONTRIBUTING.md) dla instrukcji i wymagań. Zmiany są po angielsku.

## Licencja

MIT. Zobacz plik `LICENSE` żeby zobaczyć szczegóły.
