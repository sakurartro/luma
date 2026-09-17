from pydantic import BaseModel
from datetime import datetime

class Application(BaseModel):
    name: str
    categories: list[str] | None = None
    app_path: str
    icon_path: str | None = None
    command: str
    last_used: datetime | None = None
    last_checked: datetime | None = None


class Applications(BaseModel):
    apps: list[Application]


class IconPathes(BaseModel):
    name: str
    path: str
    
class Badges(BaseModel):
    name: str
    apps: list[Application]
