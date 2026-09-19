import onnxruntime as ort
from tokenizers import Tokenizer
import numpy as np
from PIL import Image

class SigTest:
    def __init__(self):
        self.model = "models/text_model_int8.onnx"
        self.sess = ort.InferenceSession(self.model, providers=["CPUExecutionProvider"])
        self.input_name = self.sess.get_inputs()[0].name
        self.output_name = self.sess.get_outputs()[1].name
        self.tokenizer = Tokenizer.from_file("models/tokenizer.json")
        self.tokenizer.enable_padding(length=64)
        self.tokenizer.enable_truncation(max_length=64)
        self.image_path = "backend/files/ai_test/pic.png"

        self.visual_model = "models/vision_model_int8.onnx"
        self.visual_sess = ort.InferenceSession(self.visual_model, providers=["CPUExecutionProvider"])
        self.visual_input = self.visual_sess.get_inputs()[0].name
        self.visual_output = self.visual_sess.get_outputs()[1].name
        self.texts = [
            "A Pallas's cat standing in the snow",
            "A cat in the snow",
            "A dog standing in the snow",
            "A cat sitting indoors on a sofa",
            "A red sports car",
            "A bowl of fruit"
        ]

        


    def text_to_vec(self):
        encodings = self.tokenizer.encode_batch(self.texts)

        input_ids = np.array(
            [encoding.ids for encoding in encodings],
            dtype=np.int64
        )
        output = self.sess.run([self.output_name], {self.input_name: input_ids})[0]
        output = output / np.linalg.norm(output, axis=-1, keepdims=True)
        return output

    def preprocess_image(self):
        image = Image.open(self.image_path)
        image = image.convert('RGB')
        image = image.resize(
            (224, 224),
            resample=Image.Resampling.BILINEAR
        )

        x = np.asarray(image, dtype=np.float32)
        x *= 1.0 / 255.0
        x = (x - 0.5) / 0.5

        x = np.transpose(x, (2, 0, 1))

        x = np.expand_dims(x, axis=0)

        return x

    def image_to_vec(self):
        image_data = self.preprocess_image()
        output = self.visual_sess.run([self.visual_output], {self.visual_input: image_data})[0]
        return output / np.linalg.norm(output, axis=-1, keepdims=True)

    def comparison(self):
        image = self.image_to_vec()
        text = self.text_to_vec()
        scores = image @ text.T
        best_index = np.argmax(scores)

        print(self.texts[best_index])
        print(scores[0, best_index])

        

sig = SigTest()
    
if __name__ == "__main__":
    sig.comparison()
