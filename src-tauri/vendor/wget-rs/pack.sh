for dir in */; do
  # 判断每个目录下是否有wget-rs*文件
  if [ -f "${dir}wget-rs*" ]; then
    # 去掉最后的斜杠得到目录名
    foldername=${dir%/}
    # 打包为zip格式
    #zip "${foldername}.zip" "${dir}wget-rs*"
    # 如果要tar.gz格式
    tar -czvf "../${foldername}.tar.gz" -C "${dir}" wget-rs*
  fi
done