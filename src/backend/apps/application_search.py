import os
from backend.apps.icons.icon_search import icons_obj
from backend.apps.models import Application, Applications
from database.service import get_data, add_batch_of_data
from rapidfuzz import fuzz
from backend.config_parser.async_config_parser import AsyncConfigParser
import asyncio
from datetime import datetime
import subprocess

class ApplicationData:
    def __init__(self) -> None:
        self._apps: list[Application] = []
        self.refresh()

    @staticmethod
    def _application_from_row(row: dict) -> Application:
        return Application(
            name=row["name"],
            categories=row["categories"],
            app_path=row["app_path"],
            icon_path=row["icon_path"],
            command=row["command"] or "",
            last_used=datetime.fromisoformat(row["last_used"]) if row["last_used"] else None,
            last_checked=datetime.fromisoformat(row["last_checked"]) if row["last_checked"] else None,
        )

    def initial_scan(self):
        asyncio.run(self._initial_scan())

    async def _initial_scan(self):
        items: list[dict] = []
        for root, _, files in os.walk("/usr/share/applications"):
            for file in files:
                if file.endswith(".desktop"):
                    path=os.path.join(root, file)
                    desktop = await AsyncConfigParser.from_file(path)
                    icon_path = icons_obj.get_icon(path)
                    command = desktop.get_command()
                    categories = desktop.get_categories()
                    name = desktop.get_app_name()
                    items.append({
                        "name": name,
                        "categories": categories,
                        "app_path": path,
                        "icon_path": icon_path,
                        "command": command,
                    })
        if items:
            await add_batch_of_data(items)

    def refresh(self):
        self._apps = [
            self._application_from_row(row)
            for row in asyncio.run(get_data())
        ]


    def get_apps(self) -> list[Application]:
        apps = [self._application_from_row(row) for row in asyncio.run(get_data())]
        return sorted(apps, key=lambda app: app.last_used or datetime.min, reverse=True)

    def search_by_query(self, query: str) -> Applications:
        apps: list[Application] = []
        data = apps_obj._apps
        data.sort(key=lambda row: row.last_used or datetime.min, reverse=True)
        for app_data in data:
            if query in app_data.name or fuzz.partial_ratio(query.lower(), app_data.name.lower()) >= 80:
                apps.append(app_data)
        return Applications(apps=apps)

    def filter_apps_by_categories(self) -> dict[str, list[Application]]:
        result: dict[str, list[Application]] = {}
        for application in self._apps:
            for category in application.categories or []:
                result.setdefault(category, []).append(application)
        return result


apps_obj = ApplicationData()

if __name__ == "__main__":
    app = apps_obj._apps[0]
    subprocess.run([arg for arg in app.command.split(" ")])

                
