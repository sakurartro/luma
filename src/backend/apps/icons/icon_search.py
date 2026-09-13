from rapidfuzz import process, fuzz
import os
from xdg.IconTheme import getIconPath
from xdg.DesktopEntry import DesktopEntry

class IconParse:
    def icon_search_from_list(self, target: str, icon_names: list[str]) -> int | None:
        result = process.extractOne(target, icon_names, scorer=fuzz.WRatio, score_cutoff=85)
        return result[2] if result else None


    def get_icon(self, path: str):
        desktop = DesktopEntry(path)
        icon = desktop.getIcon()
        if not icon:
            return None

        if os.path.isabs(icon):
            return icon if os.path.isfile(icon) else None
        
        return getIconPath(icon)

icons_obj = IconParse()
    