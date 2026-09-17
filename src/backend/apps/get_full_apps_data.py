# import asyncio
# from backend.apps.models import DatabaseData
# from database.service import get_data
# from datetime import datetime

# async def get_full_apps_data() -> list[DatabaseData]:
#     raw_data: list[set] = await get_data()
#     apps_list: list[DatabaseData] = []
#     for app in raw_data:
#         id: int = app[0]
#         name: str = app[1]
#         categories: list[str] = app[2].split(";")
#         app_path: str = app[3]
#         icon_path: str | None = app[4]
#         command: str = app[5]
#         last_used: datetimee | None = app[6]
#         last_checked: datetime | None = app[7]
#         apps_list.append(
#             DatabaseData(
#                 id=id,
#                 name=name,
#                 categories=categories,
#                 app_path=app_path,
#                 icon_path=icon_path,
#                 command=command,
#                 last_used=last_used,
#                 last_checked=last_checked,
#             )
#         )

#     return apps_list


# if __name__ == "__main__":
#     print(asyncio.run(get_full_apps_data()))