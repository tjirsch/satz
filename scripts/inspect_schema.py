import json
import sys

filepath = sys.argv[1]

try:
    with open(filepath, "r") as f:
        data = json.load(f)

    schemas = data.get("provider_schemas", {}).get("registry.opentofu.org/hashicorp/google", {}).get("resource_schemas", {})
    if not schemas:
        # Try finding standard registry path
        schemas = data.get("provider_schemas", {}).get("registry.terraform.io/hashicorp/google", {}).get("resource_schemas", {})

    print(f"Found {len(schemas)} resources.")

    target = "google_organization_policy"
    if target in schemas:
        print(f"FOUND: {target}")
        print(json.dumps(schemas[target], indent=2))
    else:
        print(f"NOT FOUND: {target}")
        # Print similar keys
        similar = [k for k in schemas.keys() if "organization" in k or "org_policy" in k]
        print(f"Similar keys: {similar}")

except Exception as e:
    print(f"Error: {e}")
