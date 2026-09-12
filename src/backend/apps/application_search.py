import os
from backend.apps.icon_search import icon_search_from_list
from backend.apps.models import Application, Applications, IconPathes
from database.service import add_data, get_data

def get_all_icons() -> list[IconPathes]:
    icons: list[IconPathes] = []
    for root, _, files in os.walk("/usr/share/pixmaps"):
        for file in files:
            name = file.split(".")[0]
            path = os.path.join(root, file)
            icons.append(IconPathes(name=name, path=path))
    return icons

def get_apps() -> Applications:
    apps: list[Application] = []
    # icons = get_all_icons()
    # for root, _, files in os.walk("/usr/share/applications"):
    #     for file in files:
    #         if file.endswith(".desktop"):
    #             name=file.split(".desktop")[0]
    #             path=os.path.join(root, file)
    #             best_icon_index = icon_search_from_list(name, [name.name for name in icons])
    #             icon_path = None
    #             if best_icon_index is not None:
    #                 icon_path = icons[best_icon_index].path
    #             #addin to database
    #         a    add_data(name, path, icon_path)
    #             app = Application(name=name, path=path, icon_path=icon_path)
    #             apps.append(app)

    data = get_data()
    for app_data in data:
        app = Application(name=app_data[1], path=app_data[2], icon_path=app_data[3])
        apps.append(app)
    return Applications(apps=apps)


if __name__ == "__main__":
    print(get_apps())

                

