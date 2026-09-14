import os
from backend.apps.icons.icon_search import icons_obj
from backend.apps.models import Application, Applications, IconPathes
from database.service import add_data, get_data
from rapidfuzz import fuzz

class ApplicationData:

    def initial_scan(self):
        apps: list[Application] = []
        for root, _, files in os.walk("/usr/share/applications"):
            for file in files:
                if file.endswith(".desktop"):
                    name=file.split(".desktop")[0]
                    path=os.path.join(root, file)
                    icon_path = icons_obj.get_icon(path)
                    add_data(name, path, icon_path)
                    app = Application(name=name, path=path, icon_path=icon_path)
                    apps.append(app)
        return Applications(apps=apps)
    def get_apps(self) -> Applications:
        apps: list[Application] = []
        data = get_data()
        data.sort(key=lambda row: row[-1], reverse=True)
        for app_data in data:
            app = Application(name=app_data[1], path=app_data[2], icon_path=app_data[3])
            apps.append(app)
        return Applications(apps=apps)

    def search_by_query(self, query: str) -> Applications:
        apps: list[Application] = []
        data = get_data()
        data.sort(key=lambda row: row[-1], reverse=True)
        for app_data in data:
            name = app_data[1]
            if query in name or fuzz.partial_ratio(query, name) >= 80:
                app = Application(name=app_data[1], path=app_data[2], icon_path=app_data[3])
                apps.append(app)

        return Applications(apps=apps)

apps_obj = ApplicationData()

if __name__ == "__main__":
    print(apps_obj.initial_scan())

                

