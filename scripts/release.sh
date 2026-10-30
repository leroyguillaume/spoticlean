#!/usr/bin/env bash
# Cuts a release: bumps the version, regenerates what derives from it,
# commits, tags, and asks before pushing. CI publishes from the tag.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/release.sh [--chart] [--app-version <semver>] [--dry-run] <semver>

Cut a release of the application (tag vX.Y.Z), or with --chart of the Helm
chart (tag chart-X.Y.Z). The version is always given explicitly.

Options:
  --chart                 release the chart instead of the application
  --app-version <semver>  chart only: the application version the chart pins
                          (default: the latest v* tag)
  --dry-run               print what would happen and change nothing
  -h, --help              show this help

Examples:
  scripts/release.sh 0.2.0
  scripts/release.sh 0.3.0-rc1
  scripts/release.sh --chart 0.2.0
  scripts/release.sh --chart --app-version 0.2.0 --dry-run 0.2.0
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

semver='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
chart=false
dry_run=false
app_version=''
version=''

while [ $# -gt 0 ]; do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --chart) chart=true ;;
    --dry-run) dry_run=true ;;
    --app-version)
      [ $# -ge 2 ] || die "--app-version needs a value"
      app_version="$2"
      shift
      ;;
    -*) die "unknown option $1 (see --help)" ;;
    *)
      [ -z "$version" ] || die "only one version may be given"
      version="$1"
      ;;
  esac
  shift
done

[ -n "$version" ] || {
  usage >&2
  exit 1
}
[[ "$version" =~ $semver ]] || die "'$version' is not a semver version"

cd "$(git rev-parse --show-toplevel)"

chart_file=charts/spoticlean/Chart.yaml
readme=README.md
branch="$(git symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null | sed 's#^origin/##')"
[ -n "$branch" ] || die "cannot tell the default branch: run 'git remote set-head origin --auto'"

if $chart; then
  tag="chart-${version}"
  if [ -z "$app_version" ]; then
    latest="$(git tag --list 'v*' | sed 's/^v//' | sort -V | tail -n 1)"
    [ -n "$latest" ] || die "no v* tag yet: release the application first, or pass --app-version"
    app_version="$latest"
  fi
  [[ "$app_version" =~ $semver ]] || die "'$app_version' is not a semver version"
  current="$(sed -n 's/^version: *//p' "$chart_file")"
  subject="release chart ${version}"
else
  [ -z "$app_version" ] || die "--app-version only applies with --chart"
  tag="v${version}"
  current="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')"
  subject="release ${version}"
fi

echo "current version: ${current}"
echo "target version:  ${version}"
$chart && echo "appVersion:      ${app_version}"
echo "commit subject:  ${subject}"
echo "tag:             ${tag}"

if $dry_run; then
  exit 0
fi

[ -z "$(git status --porcelain)" ] || die "the working tree is not clean: commit or stash first"
[ "$(git symbolic-ref --short HEAD)" = "$branch" ] || die "not on ${branch}: check it out first"
git fetch --quiet origin
[ "$(git rev-list --count "HEAD..origin/${branch}")" = 0 ] || die "${branch} is behind origin: pull first"
! git rev-parse --verify --quiet "refs/tags/${tag}" >/dev/null || die "tag ${tag} already exists"
if $chart; then
  git rev-parse --verify --quiet "refs/tags/v${app_version}" >/dev/null ||
    die "appVersion ${app_version} has no tag v${app_version}: release the application first"
fi

if $chart; then
  sed -i.bak -E "s/^version: .*/version: ${version}/; s/^appVersion: .*/appVersion: \"${app_version}\"/" "$chart_file"
  rm -f "${chart_file}.bak"
  if [[ "$version" != *-* ]]; then
    sed -i.bak -E "s/(helm install .* --version )[0-9][0-9A-Za-z.+-]*/\1${version}/" "$readme"
    rm -f "${readme}.bak"
  fi
  helm-docs --chart-search-root charts
else
  cargo set-version "$version"
fi

git add -A
git commit --quiet --message "$subject"
git tag --annotate "$tag" --message "$tag"
echo "committed and tagged ${tag}"

if [ -t 0 ]; then
  printf 'push %s and %s to origin? this publishes the release. [y/N] ' "$branch" "$tag"
  read -r reply
else
  reply=n
fi
case "$reply" in
  y | Y) git push origin "$branch" "$tag" ;;
  *) echo "not pushed; to publish later: git push origin ${branch} ${tag}" ;;
esac
