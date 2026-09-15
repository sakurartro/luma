import os
from backend.apps.icons.icon_search import icons_obj
from backend.apps.models import Application, Applications, IconPathes
from database.service import add_data, get_data, get_all_categories, get_apps_by_category
from rapidfuzz import fuzz
from backend.config_parser.async_config_parser import AsyncConfigParser
import asyncio

class ApplicationData:

    def initial_scan(self):
        apps: list[Application] = []
        for root, _, files in os.walk("/usr/share/applications"):
            for file in files:
                if file.endswith(".desktop"):
                    path=os.path.join(root, file)
                    desktop = asyncio.run(AsyncConfigParser.from_file(path))
                    icon_path = icons_obj.get_icon(path)
                    command = desktop.get_command()
                    categories = desktop.get_categories()
                    name = desktop.get_app_name()
                    asyncio.run(add_data(name, categories, path, icon_path, command))
                    app = Application(name=name, path=path, icon_path=icon_path, categories=categories, command=command)
                    apps.append(app)
        return Applications(apps=apps)
    def get_apps(self) -> Applications:
        apps: list[Application] = []
        data = asyncio.run(get_data())
        data.sort(key=lambda row: row[6], reverse=True)
        for app_data in data:
            app = Application(name=app_data[1], path=app_data[3], icon_path=app_data[4], categories=app_data[2], command=app_data[5])
            apps.append(app)
        return Applications(apps=apps)

    def search_by_query(self, query: str) -> Applications:
        apps: list[Application] = []
        data = asyncio.run(get_data())
        data.sort(key=lambda row: row[6], reverse=True)
        for app_data in data:
            name = app_data[1]
            if query in name or fuzz.partial_ratio(query.lower(), name.lower()) >= 80:
                app = Application(name=app_data[1], path=app_data[3], icon_path=app_data[4], categories=app_data[2], command=app_data[5])
                apps.append(app)

        return Applications(apps=apps)

    def filter_apps_by_categories(self) -> dict[str, list[Application]]:
        categories = {
            cat.strip() for (cats,) in asyncio.run(get_all_categories()) if cats for cat in cats.split(";") if cat
        }
        result: dict[str, list[Application]] = {}
        for category in categories:
            data = asyncio.run(get_apps_by_category(category))
            apps = []
            for app_data in data:
                app = Application(name=app_data[1], path=app_data[3], icon_path=app_data[4], categories=app_data[2], command = app_data[5])
                apps.append(app)
            result[category] = apps
        return result


apps_obj = ApplicationData()

if __name__ == "__main__":
    print(apps_obj.filter_apps_by_categories())

                
