import numpy as np
from PIL import Image

mean = [0.48145466, 0.4578275,  0.40821073]
image_std = [0.26862954, 0.26130258, 0.27577711]

def pre_image(path: str):
    with Image.open(path) as img:
        img = img.convert("RGB")
        w, h = img.size
        if w < h:
            new_w = 224
            new_h = round(h * 224 / w)
        else:
            new_h = 224
            new_w = round(w * 224 / h)

    img = img.resize(
        (new_w, new_h),
        Image.Resampling.BICUBIC
    )

    w, h = img.size

    left = (w - 224) // 2
    top = (h - 224) // 2

    img = img.crop((left, top, left + 224, top + 224))
    
    img = np.array(img, dtype=np.float32)
    img = img / 255.0
    
    mean_arr = np.array(mean, dtype=np.float32)
    std_arr = np.array(image_std, dtype=np.float32)
    img = (img - mean_arr) / std_arr
    
    img = np.transpose(img, (2, 0, 1))
    img = np.expand_dims(img, axis=0)
    return img