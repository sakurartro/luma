import os
from backend.apps.icons.icon_search import icons_obj
from backend.apps.models import Application, Applications, IconPathes
from database.service import add_data, get_data, get_all_categories, get_apps_by_category, add_batch_of_data
from rapidfuzz import fuzz
from backend.config_parser.async_config_parser import AsyncConfigParser
import asyncio
from database.init import init_db
from datetime import datetime
import subprocess

class ApplicationData:
    def __init__(self) -> None:
        self._apps: list[Application] = []
        self.refresh()

    @staticmethod
    def _application_from_row(row: tuple) -> Application:
        return Application(
            name=row[1],
            categories=[category for category in (row[2] or "").split(";") if category],
            app_path=row[3],
            icon_path=row[4],
            command=row[5] or "",
            last_used=datetime.fromisoformat(row[6]) if row[6] else None,
            last_checked=datetime.fromisoformat(row[7]) if row[7] else None,
        )

    def initial_scan(self):
        asyncio.run(self._initial_scan())

    async def _initial_scan(self):
        items: list[tuple] = []
        for root, _, files in os.walk("/usr/share/applications"):
            for file in files:
                if file.endswith(".desktop"):
                    path=os.path.join(root, file)
                    desktop = await AsyncConfigParser.from_file(path)
                    icon_path = icons_obj.get_icon(path)
                    command = desktop.get_command()
                    categories = desktop.get_categories()
                    name = desktop.get_app_name()
                    now = datetime.now()
                    now = now.strftime("%Y-%m-%d %H:%M:%S")
                    items.append((name, categories, path, icon_path, command, now, now))
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
        categories = {
            cat.strip() for (cats,) in asyncio.run(get_all_categories()) if cats for cat in cats.split(";") if cat
        }
        result: dict[str, list[Application]] = {}
        for category in categories:
            data = asyncio.run(get_apps_by_category(category))
            result[category] = [self._application_from_row(row) for row in data]
        return result


apps_obj = ApplicationData()

if __name__ == "__main__":
    app = apps_obj._apps[0]
    subprocess.run([arg for arg in app.command.split(" ")])

                
