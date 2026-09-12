from pydantic import BaseModel

class Application(BaseModel):
    name: str
    path: str
    icon_path: str | None = None

class Applications(BaseModel):
    apps: list[Application]


class IconPathes(BaseModel):
    name: str
    path: str