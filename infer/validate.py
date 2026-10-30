"""Every verbatim field must be an exact substring of its source. Failures are
dropped (or the field emptied) and counted; nothing unverified is written."""


def validate(an, tr, cat):
    c = {"checked": 0, "failed": 0, "dropped": []}

    def ok(sub, src, where):
        c["checked"] += 1
        good = bool(sub) and sub in src
        if not good:
            c["failed"] += 1
            c["dropped"].append(f"{where}: {sub[:60]!r}")
        return good

    for j in an["jokes"]:
        w = f"#{j['idx']} {j['title']}"
        if not ok(j["transcript"], tr.text, w + " transcript"):
            j["transcript"] = ""
        j["punchlines"] = [p for p in j["punchlines"] if ok(p["text"], j["transcript"] or tr.text, w + " punchline")]
        if j["joke_id"] and j["joke_id"] not in cat.jokes:
            c["failed"] += 1; c["dropped"].append(w + " joke_id"); j["joke_id"] = None
        if j["version_unit_id"]:
            u = cat.units.get(j["version_unit_id"].split(":", 1)[1])
            if not u:
                c["failed"] += 1; j["version_unit_id"] = None; j["script_diff"] = None
            elif j["script_diff"]:
                sd = j["script_diff"]
                sd["dropped"] = [x for x in sd["dropped"] if ok(x, u["text"], w + " dropped")]
                sd["added"] = [x for x in sd["added"] if ok(x, j["transcript"], w + " added")]
        j["hint_labels"] = [h for h in j["hint_labels"] if h["label_id"] in cat.labels]
        j["similar"] = [s for s in j["similar"] if s["joke_id"] in cat.jokes]
    return c
