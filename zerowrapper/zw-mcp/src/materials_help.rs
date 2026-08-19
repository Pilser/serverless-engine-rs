pub const ACTIONS: &[&str] = &[
    "about", "list", "show", "create", "publish", "embed", "clone", "commit", "versions",
    "update", "query", "srv", "remove",
];

pub const GLOBAL_USAGE: &str =
    "mat --session <sid> <action> [args]\n\
     --session <sid> is required on publish, embed, clone, commit, update\n\
     actions:\n\
     about              what template materials are + the full lifecycle\n\
     list               list all public templates (with head version + record/file counts)\n\
     show <tpl> [--version N]          inspect a template version (default: head)\n\
     versions <tpl>     list all versions of a template\n\
     create <tpl> [--name \"<n>\"] [--kind app|card|theme|component|full-app] [--desc \"<d>\"]\n\
     publish --board <board_id> --tpl <tpl> [--name \"<n>\"] [--kind K] [--desc \"<d>\"] [--note \"<why>\"]   promote a session board to template v1\n\
     embed --tpl <tpl> [--version N]    reference a version (shared live data, no copy)\n\
     clone --tpl <tpl> --version N      copy a version into your session (files + data snapshot)\n\
     commit --tpl <tpl> [--note \"<why>\"]   publish your cloned changes as the next version\n\
     update --tpl <tpl> --version N     move your clone to a newer version\n\
     remove <tpl>       delete a template and all its versions\n\
     query <tpl> [--version N] [--what info|schema|records|count|files|hooks|rate|ttl] [--limit N] [--offset N]\n\
     srv <group> <verb> <tpl>[@ver] [args]   full serverless CLI on the exact version:\n\
        apps     update|show|schema|rate|ttl|hooks|keys|audit|link (create/delete are locked)\n\
        records  submit|list|get|update|patch|query|search|count|aggregate|delete|audit|call\n\
        files    list|get\n\
        (same verbs/help as 'wb ... srv', e.g. mat srv apps hooks --help)\n\
     --help | <action> --help\n";

pub fn action_help(action: &str) -> &'static str {
    match action {
        "about" => "mat --session <sid> about\n\nprints the full capability summary: templates as reusable public serverless apps (embed = shared live data, clone = snapshot, commit = next version with changelog). Run it first.\n",
        "list" => "mat list\n\nlists all public templates with their head version, record and file counts.\n",
        "show" => "mat show <template_id> [--version N]\n\nprints a template version: manifest, schema, public_reads, unique_key, rate, ttl, hooks, record count, files, changelog and diff (default version = head).\n",
        "versions" => "mat versions <template_id>\n\nlists every published version of a template.\n",
        "create" => "mat create <template_id> [--name \"<n>\"] [--kind app|card|theme|component|full-app] [--desc \"<d>\"]\n\ncreates an empty template v1 (no data/files) that later versions can be committed against.\n",
        "publish" => "mat --session <sid> publish --board <board_id> --tpl <template_id> [--name \"<n>\"] [--kind K] [--desc \"<d>\"] [--note \"<why>\"]\npromotes a serverless board inside your session to a new public template v1 (copies config, records, hooks and files). The template must not exist yet — use commit to add later versions.\n",
        "embed" => "mat --session <sid> embed --tpl <template_id> [--version N]\n\nembeds a reference to a template version with SHARED live data (no copy, no session board created). Reading/writing goes to the version's own DB.\n",
        "clone" => "mat --session <sid> clone --tpl <template_id> --version N\n\ncopies a template version into your session: files into sessions/<sid>/materials/<tpl>@<ver>/files and a snapshot of config + data into a board mat:<tpl>@<ver>. The clone owns its data and can be committed back.\n",
        "commit" => "mat --session <sid> commit --tpl <template_id> [--note \"<why>\"]\n\npublishes your cloned board as the next version (vN+1): files + config + data snapshot, plus a changelog.md and diff.json against the previous version.\n",
        "update" => "mat --session <sid> update --tpl <template_id> --version N\n\nre-points your clone to a newer version: replaces files and re-snapshots config + data from that version.\n",
        "remove" => "mat remove <template_id>\n\npermanently deletes a template: manifest, all versions, version DBs and its system row.\n",
        "query" => "mat query <template_id> [--version N] [--what info|schema|records|count|files|hooks|rate|ttl] [--limit N] [--offset N]\n\nreads data from an exact template version. Default what=info, version=head.\n",
        "srv" => "mat srv <group> <verb> <tpl>[@ver] [args]\n\nfull serverless CLI against the exact version (build the template's app in place, exactly like 'wb ... srv' but scoped to {tpl}@{ver}).\napps: show|schema|update|rate|ttl|hooks|keys|audit|link (create/delete locked: the version owns one app).\nrecords: submit|list|get|update|patch|query|search|count|aggregate|delete|audit|call — submit validates against the schema, search is full-text BM25, query supports filters/order/pagination, aggregate supports sum/avg/min/max/count + group_by, call is the outbound HTTP client with SSRF guard.\nfiles: list|get.\nAny <verb> --help gives the full detail, e.g. mat srv records --help or mat srv apps hooks --help.\n",
        _ => GLOBAL_USAGE,
    }
}

pub const ABOUT_TEXT: &str =
    "Template materials are PUBLIC, VERSIONED, reusable serverless apps.\n\
     \n\
     Every template version is an isolated serverless board addressed as <template_id>@<version>.\n\
     Versions are immutable; they live under templates/<template_id>/versions/<version>/ with a\n\
     manifest.json, changelog.md (why a version exists) and diff.json (what changed), plus files/.\n\
     The version's own DB holds its schema and records (sessions/<tpl>@<ver>/<tpl>@<ver>.db).\n\
     \n\
     Lifecycle:\n\
     - create  -> an empty template v1 (a name, kind and description in the public registry).\n\
     - publish -> promote one of YOUR session boards to a new template v1 (config + data + files).\n\
     - embed   -> reference a version WITHOUT copying: shared live data. No own board, no files.\n\
     - clone   -> copy a version into your session: your own files copy + data snapshot in a board.\n\
     - commit  -> publish your clone's changes as the next version (vN+1) with a changelog.\n\
     - update  -> move an existing clone/embed to a newer version.\n\
     \n\
     Rule of thumb: embed if you only need to point at shared data; clone if the data is YOURS;\n\
     commit if you improved a clone and want the world to have it.\n\
     Every material reference is pinned to <template_id>@<version> so dependents never break.";
