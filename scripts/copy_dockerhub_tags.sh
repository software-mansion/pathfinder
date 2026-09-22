#!/usr/bin/env bash

set -euo pipefail

usage() {
    cat <<'EOF'
Copy tags between two public Docker Hub repositories and verify their digests.

Usage:
  copy_dockerhub_tags.sh [options] SOURCE_REPOSITORY DESTINATION_REPOSITORY

Arguments:
  SOURCE_REPOSITORY       Docker Hub repository to copy from, e.g. eqlabs/pathfinder
  DESTINATION_REPOSITORY  Docker Hub repository to copy to, e.g. swmansion/pathfinder

Options:
  --execute               Perform the copy. Without this flag, only list matching tags.
  --tag-regex REGEX       Copy only tags matching this Bash regular expression.
                          By default, every tag is copied.
  --order-by-source-updated
                          Copy tags in source update order, oldest first. This
                          reproduces the source repository's relative tag order
                          in the destination repository. Existing destination
                          tags are forcibly re-tagged to update their ordering.
  --fail-fast             Stop after the first copy or verification failure.
                          Required when executing with
                          --order-by-source-updated so that a later retry does
                          not change the intended tag order.
  --start-at TAG          Start at this tag (inclusive) in the sorted tag list.
                          Useful for resuming an interrupted migration.
  --yes                   Do not ask for confirmation when used with --execute.
  -h, --help              Show this help message.

Requirements:
  curl, jq, and crane

Authenticate to the destination namespace before executing the copy. Use a
Docker Hub access token as the password (not the account password):
  printf '%s' "$DOCKER_HUB_ACCESS_TOKEN" | \
    crane auth login index.docker.io \
      --username "$DOCKER_HUB_USERNAME" --password-stdin

Examples:
  scripts/copy_dockerhub_tags.sh eqlabs/pathfinder swmansion/pathfinder
  scripts/copy_dockerhub_tags.sh --execute eqlabs/pathfinder swmansion/pathfinder
  scripts/copy_dockerhub_tags.sh --execute --start-at v0.20.0 \
    eqlabs/pathfinder swmansion/pathfinder
  scripts/copy_dockerhub_tags.sh --execute --yes \
    --order-by-source-updated --fail-fast \
    eqlabs/pathfinder swmansion/pathfinder
  scripts/copy_dockerhub_tags.sh --execute --tag-regex '^v0\.2[3-4]\.' \
    eqlabs/pathfinder swmansion/pathfinder
EOF
}

execute=false
assume_yes=false
order_by_source_updated=false
fail_fast=false
tag_regex=''
start_at=''
repositories=()

while (($# > 0)); do
    case "$1" in
        --execute)
            execute=true
            shift
            ;;
        --tag-regex)
            if (($# < 2)); then
                echo "Error: --tag-regex requires a value." >&2
                usage >&2
                exit 2
            fi
            tag_regex=$2
            shift 2
            ;;
        --order-by-source-updated)
            order_by_source_updated=true
            shift
            ;;
        --fail-fast)
            fail_fast=true
            shift
            ;;
        --start-at)
            if (($# < 2)); then
                echo "Error: --start-at requires a value." >&2
                usage >&2
                exit 2
            fi
            start_at=$2
            shift 2
            ;;
        --yes)
            assume_yes=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --*)
            echo "Error: Unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
        *)
            repositories+=("$1")
            shift
            ;;
    esac
done

if ((${#repositories[@]} != 2)); then
    echo "Error: Expected a source and a destination repository." >&2
    usage >&2
    exit 2
fi

source_repository=${repositories[0]}
destination_repository=${repositories[1]}
repository_pattern='^[a-z0-9]+([._-][a-z0-9]+)*/[a-z0-9]+([._-][a-z0-9]+)*$'

if [[ ! $source_repository =~ $repository_pattern ]]; then
    echo "Error: Invalid source repository: $source_repository" >&2
    echo "Expected a Docker Hub repository in namespace/name format." >&2
    exit 2
fi

if [[ ! $destination_repository =~ $repository_pattern ]]; then
    echo "Error: Invalid destination repository: $destination_repository" >&2
    echo "Expected a Docker Hub repository in namespace/name format." >&2
    exit 2
fi

if [[ $source_repository == "$destination_repository" ]]; then
    echo "Error: Source and destination repositories must be different." >&2
    exit 2
fi

if [[ $execute == true && $order_by_source_updated == true && $fail_fast != true ]]; then
    echo "Error: --order-by-source-updated requires --fail-fast when executing." >&2
    echo "This prevents retries from changing the intended tag order." >&2
    exit 2
fi

for command_name in curl jq; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Error: Required command is not installed: $command_name" >&2
        exit 1
    fi
done

if [[ $execute == true ]] && ! command -v crane >/dev/null 2>&1; then
    echo "Error: Required command is not installed: crane" >&2
    exit 1
fi

temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/copy-dockerhub-tags.XXXXXX")
trap 'rm -rf "$temporary_directory"' EXIT

all_tags_file="$temporary_directory/all-tags.txt"
all_tags_with_updated_file="$temporary_directory/all-tags-with-updated.tsv"
selected_tags_file="$temporary_directory/selected-tags.txt"
failures_file="$temporary_directory/failures.txt"

fetch_tags() {
    local repository=$1
    local names_file=$2
    local updated_file=$3
    local page_file
    local next_url

    : >"$names_file"
    : >"$updated_file"
    next_url="https://hub.docker.com/v2/repositories/${repository}/tags?page_size=100&ordering=name"

    while [[ -n $next_url && $next_url != "null" ]]; do
        page_file="$temporary_directory/page.json"
        curl --fail --silent --show-error --location "$next_url" >"$page_file"
        jq -r '.results[].name' "$page_file" >>"$names_file"
        jq -r '.results[] | [.name, .last_updated] | @tsv' "$page_file" \
            >>"$updated_file"
        next_url=$(jq -r '.next // empty' "$page_file")
    done
}

sort_tags_by_updated() {
    local updated_file=$1
    local ordered_names_file=$2

    if grep -q $'\tnull$' "$updated_file"; then
        echo "Error: Docker Hub did not return last_updated for every tag." >&2
        return 1
    fi

    LC_ALL=C sort -t $'\t' -k2,2 -k1,1 "$updated_file" \
        | cut -f1 >"$ordered_names_file"

    if [[ $(sort -u "$ordered_names_file" | wc -l | tr -d ' ') \
        != $(wc -l <"$ordered_names_file" | tr -d ' ') ]]; then
        echo "Error: Docker Hub returned a tag more than once while paginating." >&2
        echo "Ensure that no images are being published and try again." >&2
        return 1
    fi
}

echo "Fetching tags from $source_repository..."
fetch_tags "$source_repository" "$all_tags_file" "$all_tags_with_updated_file"

if [[ $order_by_source_updated == true ]]; then
    sort_tags_by_updated "$all_tags_with_updated_file" "$all_tags_file"
else
    sort -u "$all_tags_file" -o "$all_tags_file"
fi

start_reached=true
if [[ -n $start_at ]]; then
    start_reached=false
fi

while IFS= read -r tag; do
    if [[ -n $tag_regex && ! $tag =~ $tag_regex ]]; then
        continue
    fi

    if [[ $start_reached != true ]]; then
        if [[ $tag != "$start_at" ]]; then
            continue
        fi
        start_reached=true
    fi

    printf '%s\n' "$tag" >>"$selected_tags_file"
done <"$all_tags_file"

if [[ $start_reached != true ]]; then
    echo "Start tag was not found in the selected tag list: $start_at" >&2
    exit 1
fi

if [[ ! -s $selected_tags_file ]]; then
    echo "No tags matched the requested selection." >&2
    exit 1
fi

tag_count=$(wc -l <"$selected_tags_file" | tr -d ' ')
echo "Selected $tag_count tag(s):"
sed 's/^/  - /' "$selected_tags_file"

if [[ $execute != true ]]; then
    echo
    echo "Dry run only. Re-run with --execute to copy these tags."
    exit 0
fi

if [[ $assume_yes != true ]]; then
    echo
    echo "Existing tags with the same names in $destination_repository may be overwritten."
    read -r -p "Continue? [y/N] " answer
    if [[ $answer != "y" && $answer != "Y" ]]; then
        echo "Aborted."
        exit 0
    fi
fi

copied_count=0
stopped_at=''

while IFS= read -r tag; do
    source_reference="$source_repository:$tag"
    destination_reference="$destination_repository:$tag"

    echo
    echo "Copying $source_reference -> $destination_reference"

    if ! source_digest=$(crane digest "$source_reference"); then
        echo "$tag: failed to read source digest" | tee -a "$failures_file" >&2
        if [[ $fail_fast == true ]]; then
            stopped_at=$tag
            break
        fi
        continue
    fi

    if ! crane cp "$source_reference" "$destination_reference"; then
        echo "$tag: copy failed" | tee -a "$failures_file" >&2
        if [[ $fail_fast == true ]]; then
            stopped_at=$tag
            break
        fi
        continue
    fi

    if ! destination_digest=$(crane digest "$destination_reference"); then
        echo "$tag: failed to read destination digest" | tee -a "$failures_file" >&2
        if [[ $fail_fast == true ]]; then
            stopped_at=$tag
            break
        fi
        continue
    fi

    if [[ $source_digest != "$destination_digest" ]]; then
        echo "$tag: digest mismatch ($source_digest != $destination_digest)" \
            | tee -a "$failures_file" >&2
        if [[ $fail_fast == true ]]; then
            stopped_at=$tag
            break
        fi
        continue
    fi

    # crane cp skips the manifest PUT when an existing destination tag already
    # has the requested digest. Force that PUT so Docker Hub updates the tag's
    # last_updated value and the requested relative ordering is reproduced.
    if [[ $order_by_source_updated == true ]]; then
        if ! crane tag "$destination_repository@$source_digest" "$tag"; then
            echo "$tag: failed to force destination tag update" \
                | tee -a "$failures_file" >&2
            if [[ $fail_fast == true ]]; then
                stopped_at=$tag
                break
            fi
            continue
        fi

        if ! destination_digest=$(crane digest "$destination_reference"); then
            echo "$tag: failed to verify forced destination tag update" \
                | tee -a "$failures_file" >&2
            if [[ $fail_fast == true ]]; then
                stopped_at=$tag
                break
            fi
            continue
        fi

        if [[ $source_digest != "$destination_digest" ]]; then
            echo "$tag: digest mismatch after forced tag update ($source_digest != $destination_digest)" \
                | tee -a "$failures_file" >&2
            if [[ $fail_fast == true ]]; then
                stopped_at=$tag
                break
            fi
            continue
        fi
    fi

    echo "Verified digest: $source_digest"
    ((copied_count += 1))
done <"$selected_tags_file"

echo
echo "Copied and verified $copied_count of $tag_count tag(s)."

if [[ -s $failures_file ]]; then
    echo "Failures:" >&2
    sed 's/^/  - /' "$failures_file" >&2
    if [[ -n $stopped_at ]]; then
        echo >&2
        echo "Stopped at $stopped_at. Resume with: --start-at $stopped_at" >&2
    fi
    exit 1
fi

if [[ $order_by_source_updated == true ]]; then
    destination_tags_file="$temporary_directory/destination-tags.txt"
    destination_tags_with_updated_file="$temporary_directory/destination-tags-with-updated.tsv"
    destination_tags_ordered_file="$temporary_directory/destination-tags-ordered.txt"

    echo
    echo "Verifying destination tag order..."
    fetch_tags "$destination_repository" \
        "$destination_tags_file" "$destination_tags_with_updated_file"
    sort_tags_by_updated \
        "$destination_tags_with_updated_file" "$destination_tags_ordered_file"

    if ! cmp -s "$all_tags_file" "$destination_tags_ordered_file"; then
        echo "Error: Destination tag order does not match the source repository." >&2
        diff -u "$all_tags_file" "$destination_tags_ordered_file" >&2 || true
        exit 1
    fi

    echo "Verified that the complete destination tag order matches the source repository."
fi

echo "Migration completed successfully."
