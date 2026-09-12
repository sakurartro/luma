from rapidfuzz import process, fuzz

def icon_search_from_list(target: str, icon_names: list[str]) -> int | None:
    result = process.extractOne(target, icon_names, scorer=fuzz.WRatio, score_cutoff=85)
    return result[2] if result else None