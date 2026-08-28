#!/usr/bin/env python3
import argparse
import json
import os
import sys

# Try to import ruamel.yaml for comment preservation
try:
    from ruamel.yaml import YAML
except ImportError:
    print("ruamel.yaml not found. Please run with 'uv run --with ruamel.yaml scripts/update_discovery_config.py ...'")
    sys.exit(1)


def load_schemas(schema_dir):
    resources = set()
    if not os.path.exists(schema_dir):
        print(f"Schema directory {schema_dir} does not exist.")
        return resources

    for filename in os.listdir(schema_dir):
        if filename.endswith(".json"):
            path = os.path.join(schema_dir, filename)
            try:
                with open(path, "r") as f:
                    data = json.load(f)
                    for provider in data.get("provider_schemas", {}).values():
                        for resource_name in provider.get("resource_schemas", {}).keys():
                            resources.add(resource_name)
            except Exception as e:
                print(f"Error loading {filename}: {e}")
    return resources


def update_config(config_path, schema_resources):
    yaml = YAML()
    yaml.preserve_quotes = True
    yaml.indent(mapping=2, sequence=4, offset=2)

    if not os.path.exists(config_path):
        print(f"Config file {config_path} does not exist.")
        return

    with open(config_path, "r") as f:
        config = yaml.load(f)

    if "resource_types" not in config:
        config["resource_types"] = {}

    current_resources = config["resource_types"]
    added_count = 0

    for res in sorted(schema_resources):
        if res not in current_resources:
            # Add new resource
            print(f"Adding new resource: {res}")
            entry = {
                "description": f"Auto-generated entry for {res}",
                "import": False,
                "asset_type": "TODO/UNKNOWN",  # Placeholder
                "content_type": "RESOURCE",
                "derive_yaml_key_from": "name",
            }

            # Special heuristics
            if res == "google_org_policy_policy":
                entry["asset_type"] = "orgpolicy.googleapis.com/Policy"
                entry["description"] = "Organization Policy V2"
                # Enable import by default for this requested type?
                # User said "inclined to support org_policy_policy only".
                # Maybe default import to True? Let's leave False for safety unless specified.

            if res.startswith("google_folder_organization_policy"):
                # Update comment or description?
                pass

            current_resources[res] = entry
            added_count += 1

    if added_count > 0:
        print(f"Added {added_count} new resources to {config_path}")
        with open(config_path, "w") as f:
            yaml.dump(config, f)
    else:
        print("No new resources found in schemas.")


def main():
    parser = argparse.ArgumentParser(description="Update discovery-config.yaml from Terraform schemas.")
    parser.add_argument("--schema-dir", required=True, help="Directory containing provider schema JSON files.")
    parser.add_argument("--config-file", required=True, help="Path to discovery-config.yaml.")

    args = parser.parse_args()

    resources = load_schemas(args.schema_dir)
    print(f"Loaded {len(resources)} resources from schemas.")

    update_config(args.config_file, resources)


if __name__ == "__main__":
    main()
