import aiofiles
import configparser
import asyncio

class AsyncConfigParser:
    def __init__(self, path: str, config: configparser.ConfigParser) -> None:
        self.path = path
        self.config = config

    @classmethod
    async def from_file(cls, path: str):
        async with aiofiles.open(path, "r", encoding="utf-8") as f:
            data = await f.read()
        config = configparser.ConfigParser(interpolation=None)
        config.optionxform = str
        config.read_string(data)
        return cls(path, config)

    def get_app_name(self) -> str:
        value = self.config["Desktop Entry"].get("Name", "")
        return value

    def get_categories(self) -> str:
        value = self.config["Desktop Entry"].get("Categories", "")
        return value
    
    def get_command(self) -> str:
        value = self.config["Desktop Entry"].get("Exec", "")
        return value

    
if __name__ == "__main__":
    desktop = asyncio.run(AsyncConfigParser.from_file("/usr/share/applications/org.telegram.desktop.desktop"))
    print("tg_name", desktop.get_app_name(), "\n")
    print("categories", desktop.get_categories())


        

