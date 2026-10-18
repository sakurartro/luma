// Макет новых рядов поиска файлов (имя / OCR-сниппет / CLIP). Не подключён к приложению.
// Запуск: cargo run --example mockup

use slint::slint;

slint::slint! {
export global Palette {
    in property <color> bg: #0E120B;
    in property <color> surface: #151B10;
    in property <color> hover: #1E2716;
    in property <color> selected: #28331C;
    in property <color> border: #2C3620;
    in property <color> accent: #A9B665;
    in property <color> text: #E4E7D0;
    in property <color> text-dim: #7E8A6C;
}

component FileRow inherits Rectangle {
    in property <image> thumb;
    in property <bool> has-thumb: true;
    in property <bool> semantic: false;
    in property <bool> selected: false;
    in property <string> name;
    in property <string> path;
    in property <string> meta;

    height: 54px;
    border-radius: 6px;
    background: selected ? Palette.selected : transparent;
    border-width: selected ? 1px : 0px;
    border-color: Palette.accent;

    HorizontalLayout {
        padding-left: 10px;
        padding-right: 12px;
        spacing: 10px;

        if semantic: Text {
            text: "\u{f005}";
            color: Palette.accent;
            opacity: 0.5;
            width: 14px;
            font-size: 12px;
            vertical-alignment: center;
        }

        Rectangle {
            width: 38px;
            height: 38px;
            border-radius: 6px;
            background: Palette.surface;
            border-width: 1px;
            border-color: Palette.border;
            clip: true;

            if has-thumb: Image {
                source: thumb;
                x: (parent.width - self.width) / 2;
                y: (parent.height - self.height) / 2;
                width: parent.width;
                height: parent.height;
                image-fit: cover;
            }
            if !has-thumb: Image {
                source: @image-url("/home/sakura/projects/luma_rust/src/frontend/assets/file.svg");
                colorize: Palette.accent;
                x: (parent.width - self.width) / 2;
                y: (parent.height - self.height) / 2;
                width: 22px;
                height: 22px;
            }
        }

        Rectangle {
            horizontal-stretch: 1;
            VerticalLayout {
                alignment: center;
                spacing: 2px;

                Text {
                    text: name;
                    color: Palette.text;
                    font-size: 13px;
                    height: 18px;
                    overflow: elide;
                }
                Text {
                    text: path;
                    color: Palette.text-dim;
                    font-size: 12px;
                    height: 16px;
                    overflow: elide;
                }
            }
        }

        Text {
            text: meta;
            color: Palette.text-dim;
            font-size: 12px;
            vertical-alignment: center;
        }
    }
}

component SnippetRow inherits Rectangle {
    in property <image> thumb;
    in property <bool> selected: false;
    in property <string> name;
    in property <string> pre;
    in property <string> hl;
    in property <string> post;
    in property <string> meta;

    height: 66px;
    border-radius: 6px;
    background: selected ? Palette.selected : transparent;
    border-width: selected ? 1px : 0px;
    border-color: Palette.accent;

    HorizontalLayout {
        padding-left: 10px;
        padding-right: 12px;
        spacing: 10px;

        Rectangle {
            width: 38px;
            height: 38px;
            border-radius: 6px;
            background: Palette.surface;
            border-width: 1px;
            border-color: Palette.border;
            clip: true;

            Image {
                source: thumb;
                x: (parent.width - self.width) / 2;
                y: (parent.height - self.height) / 2;
                width: parent.width;
                height: parent.height;
                image-fit: cover;
            }
        }

        Rectangle {
            horizontal-stretch: 1;
            VerticalLayout {
                alignment: center;
                spacing: 2px;

                Text {
                    text: name;
                    color: Palette.text;
                    font-size: 13px;
                    height: 18px;
                    overflow: elide;
                }
                HorizontalLayout {
                    height: 16px;
                    spacing: 0px;

                    Text {
                        text: pre;
                        color: Palette.text-dim;
                        font-size: 12px;
                        width: 40px;
                        horizontal-alignment: right;
                        overflow: elide;
                    }
                    Text {
                        text: hl;
                        color: Palette.accent;
                        font-size: 12px;
                    }
                    Text {
                        text: post;
                        color: Palette.text-dim;
                        font-size: 12px;
                        horizontal-stretch: 1;
                        overflow: elide;
                    }
                }
            }
        }

        Text {
            text: meta;
            color: Palette.text-dim;
            font-size: 12px;
            vertical-alignment: center;
        }
    }
}

export component Mockup inherits Window {
    title: "Luma mockup";
    width: 760px;
    background: transparent;
    default-font-family: "JetBrainsMono Nerd Font";

    Rectangle {
        background: Palette.bg;
        border-width: 2px;
        border-color: Palette.border;
        border-radius: 10px;

        VerticalLayout {
            padding: 14px;
            spacing: 10px;

            // search bar
            Rectangle {
                height: 52px;
                border-radius: 8px;
                background: Palette.surface;
                border-width: 1px;
                border-color: Palette.border;

                HorizontalLayout {
                    padding-left: 14px;
                    padding-right: 14px;
                    spacing: 10px;

                    Image {
                        source: @image-url("/home/sakura/projects/luma_rust/src/frontend/assets/search.svg");
                        colorize: Palette.accent;
                        width: 18px;
                        height: 18px;
                        y: (parent.height - self.height) / 2;
                    }

                    Text {
                        text: "договор №45";
                        color: Palette.text;
                        font-size: 17px;
                        vertical-alignment: center;
                    }
                    Text {
                        text: "▌";
                        color: Palette.accent;
                        font-size: 17px;
                        vertical-alignment: center;
                    }
                    Rectangle { horizontal-stretch: 1; }
                }
            }

            Text {
                text: "Files";
                color: Palette.text-dim;
                font-size: 12px;
                height: 16px;
            }

            // точный хит в содержимом (OCR/PDF) — выбран
            SnippetRow {
                selected: true;
                thumb: @image-url("/home/sakura/projects/luma_rust/src/frontend/assets/file_0000000070c0820abb8763205e48e493.png");
                name: "report_final_v2.pdf";
                pre: "...итого ";
                hl: "договор №45";
                post: " подписан обеими сторонами...";
                meta: "12 окт · 2.4 МБ";
            }

            // CLIP-совпадение: фото без текста
            FileRow {
                semantic: true;
                thumb: @image-url("/home/sakura/Downloads/Telegram Desktop/IMG_20260924_201405432.jpg");
                name: "IMG_20260924_201405432.jpg";
                path: "~/Downloads/Telegram Desktop";
                meta: "24 сен · 2.0 МБ";
            }

            // имя-совпадение (fuzzy)
            FileRow {
                has-thumb: false;
                name: "Договор_аренды_квартиры.pdf";
                path: "~/Documents/2024";
                meta: "03 фев · 820 КБ";
            }

            // CLIP-совпадение: скриншот
            FileRow {
                semantic: true;
                thumb: @image-url("/home/sakura/Pictures/screenshot-2026-10-10_17-01-04.png");
                name: "screenshot-2026-10-10_17-01-04.png";
                path: "~/Pictures";
                meta: "10 окт · 1.1 МБ";
            }

            Rectangle { height: 2px; }

            // footer hint bar
            Rectangle {
                height: 30px;
                border-radius: 6px;
                background: Palette.surface;
                border-width: 1px;
                border-color: Palette.border;

                HorizontalLayout {
                    padding-left: 12px;
                    padding-right: 12px;

                    Text {
                        text: "Enter: open  |  Alt+2: files  |  Esc: close";
                        color: Palette.text-dim;
                        font-size: 12px;
                        vertical-alignment: center;
                    }
                    Rectangle { horizontal-stretch: 1; }
                    Text {
                        text: "Luma";
                        color: Palette.accent;
                        font-size: 12px;
                        vertical-alignment: center;
                    }
                }
            }
        }
    }
}
}

fn main() {
    let win = Mockup::new().unwrap();
    win.run().unwrap();
}
