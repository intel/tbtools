#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-only
# Copyright (c) 2017 Intel Corporation
# @Desc Authorize thunderbolt devices and list detailed stuffs under tbt devices
# Author: Pengfei Xu <pengfei.xu@intel.com>
# Created date: July. 27, 2017

CARGO_PATH="/root/.cargo/bin:~/.cargo/bin"
export PATH=${PATH}:$CARGO_PATH:./

TIME_FMT="%Y%m%d-%H%M%S.%N"
readonly TBAUTH="tbauth"
readonly TBLIST="tblist"
TBT_DEV_FILE="/tmp/tbt_name.txt"
TBT_PATH="/sys/bus/thunderbolt/devices"
REGEX_DEVICE="-"
REGEX_DOMAIN="domain"
DEV_FILE="/tmp/tbt_dev"
DEV_LIST="/tmp/dev_list"
export DEVICE_FILES="authorized device device_name link_speed link_width nvm_authenticate nvm_version uevent unique_id vendor vendor_name power/control"
export POWER_FILES="power/control power/runtime_status power/runtime_enabled"
export DOMAIN_FILES="security uevent"
AUTHORIZE_FILE="authorized"
TOPO_FILE="/tmp/tbt_topo.txt"
PCI_PATH="/sys/bus/pci/devices"
HOST_EXCLUDE="\-0"
PCI_HEX_FILE="/tmp/pci_hex.txt"
PCI_DEC_FILE="/tmp/pci_dec.txt"
PCI_HEX=""
PCI_DEC=""
DEV_TYPE=""
DEV_SERIAL=""
DEV_PCI=""
TBT_DEV_NAME=""
STUFF_FILE="/tmp/tbt_stuff.txt"
TBT_STUFF_LIST="/tmp/tbt_stuff_list.txt"
PF_BIOS=""
TBT_NUM=""
ROOT_PCI=""
SYS_PATH=""
VERBOSE=""

usage()
{
  cat <<-EOF >&2
  usage: ./${0##*/} [-v verbose] [-h help] [-s only show topo] Or no need parameter
EOF
  exit 0
}

test_print_trc()
{
  log_info=$1      # trace information

  echo "|$(date +"$TIME_FMT")|TRACE|$log_info|"
  echo "|$(date +"$TIME_FMT")|TRACE|$log_info|" >> /root/test_tbt_1.log
}

tbt_us_pci()
{
  local domianx=$1
  local pcis=""
  local pci=""
  local pci_us=""
  local pci_content=""
  local dr_pci_h=""
  local dr_pci_d=""

  [[ -n "$ROOT_PCI" ]] || {
    echo "Could not find tbt root PCI, exit!"
    exit 2
  }

  # dr_pci_h: BUS /devices/pci0000:00/0000:00:0d.3/domain1 -> 00
  dr_pci_h=$(udevadm info -q path --path=${TBT_PATH}/domain"$domianx" \
            | awk -F "/" '{print $(NF-1)}' \
            | cut -d ':' -f 2)
  dr_pci_d=$((0x"$dr_pci_h"))
  pcis=$(ls -1 $PCI_PATH)
  for pci in $pcis; do
    pci_us=""
    PCI_HEX=""
    PCI_DEC=""
    pci_content=$(ls -ltra "$PCI_PATH"/"$pci")
    [[ "$pci_content" == *"$ROOT_PCI"* ]] || continue

    pci_us=$(lspci -v -s "$pci" | grep -i upstream)
    if [[ -z "$pci_us" ]]; then
      continue
    else
      # For debug
      # echo "Upstream pci:$pci"
      PCI_HEX=$(echo "$pci" | cut -d ':' -f 2)
      PCI_DEC=$((0x"$PCI_HEX"))
      # Due to ICL tbt driver PCI 00:0d.2 and 00:0d.3
      # ICL no impact, due to ICL dr pci is 00
      [[ "$PCI_DEC" -gt "$dr_pci_d" ]] || {
        #echo "$PCI_DEC not greater than 3, skip"
        continue
      }
      echo "$PCI_HEX" >> $PCI_HEX_FILE
      echo "$PCI_DEC" >> $PCI_DEC_FILE
    fi
  done

  # As follow is for debug
  #echo "TBT device upstream PCI in hex:"
  #cat $PCI_HEX_FILE
  #echo "TBT device upstream PCI in dec:"
  #cat $PCI_DEC_FILE
}

# Integrated TBT root PCI will be changed so need to verify and find it
# Input: $1 TBT_ROOT_PCI
# Return 0 for true, otherwise false or block_test
verify_tbt_root_pci() {
  local root_pci=$1
  local dev_path="/sys/devices/pci0000:00"
  local result=""
  local pf=""
  local pf_name=""
  local tbt_dev=""

  # get like 1-3
  tbt_dev=$(ls ${TBT_PATH} \
              | grep "$REGEX_DEVICE" \
              | grep -v "$HOST_EXCLUDE" \
              | grep -v ":" \
              | awk '{ print length(), $0 | "sort -n" }' \
              | cut -d ' ' -f 2 \
              | head -n1)

  pf=$(dmidecode --type bios \
            | grep Version \
            | cut -d ':' -f 2)
  pf_name=$(echo ${pf: 1: 4})

  result=$(ls -1 $dev_path/"$root_pci" | grep "0000" | grep "07")

  if [[ -z "$result" ]]; then
    [[ "$tbt_dev" == *"-1"* ]] && {
      [[ "$root_pci" == *"0d.2" ]] && ROOT_PCI="0000:00:07.0"
      [[ "$root_pci" == *"0d.3" ]] && ROOT_PCI="0000:00:07.2"
    }
    [[ "$tbt_dev" == *"-3"* ]] && {
      [[ "$root_pci" == *"0d.2" ]] && ROOT_PCI="0000:00:07.1"
      [[ "$root_pci" == *"0d.3" ]] && ROOT_PCI="0000:00:07.3"
    }
    SYS_PATH="$dev_path/$ROOT_PCI"
    test_print_trc "Discrete or FW CM on $pf_name,ROOT_PCI:$ROOT_PCI, SYS_PATH:$SYS_PATH"
  elif [[ "$tbt_dev" == *"-1"* ]]; then
    ROOT_PCI=$(ls -1 $dev_path/"$root_pci" \
                  | grep "0000" \
                  | grep "07" \
                  | head -n 2 \
                  | head -n 1 \
                  | awk -F "pci:" '{print $2}')
    SYS_PATH="$dev_path/$ROOT_PCI"
    test_print_trc "Integrated on $pf_name, $tbt_dev $root_pci -> $ROOT_PCI"
  elif [[ "$tbt_dev" == *"-3"* ]]; then
    ROOT_PCI=$(ls -1 $dev_path/"$root_pci" \
                  | grep "0000" \
                  | grep "07" \
                  | head -n 2 \
                  | tail -n 1\
                  | awk -F "pci:" '{print $2}')
    SYS_PATH="$dev_path/$ROOT_PCI"
    test_print_trc "Integrated on $pf_name, $tbt_dev $root_pci -> $ROOT_PCI"
  elif [[ -z "$tbt_dev" ]]; then
    [[ "$root_pci" == *"0d.2" ]] && ROOT_PCI="0000:00:07.0"
    [[ "$root_pci" == *"0d.3" ]] && ROOT_PCI="0000:00:07.2"
    SYS_PATH="$dev_path/$ROOT_PCI"
  else
    die "Invalid tbt device sysfs:$tbt_dev"
  fi
  test_print_trc "TBT ROOT:$ROOT_PCI SYS_PATH:$dev_path/$ROOT_PCI"
}

find_root_pci()
{
  local tbt_devs=""
  local pf_name=""
  local tbt_dev=""

  # get like 1-3 tbt sysfs folder
  tbt_dev=$(ls ${TBT_PATH} \
              | grep "$REGEX_DEVICE" \
              | grep -v "$HOST_EXCLUDE" \
              | grep -v ":" \
              | awk '{ print length(), $0 | "sort -n" }' \
              | cut -d ' ' -f 2 \
              | head -n1)

  [[ -z "$tbt_dev" ]] && tbt_dev="0-0"

  pf_name=$(dmidecode --type bios \
                 | grep Version \
                 | cut -d ':' -f 2 \
                 | cut -d '.' -f 1)

  ROOT_PCI=$(udevadm info --attribute-walk --path=/sys/bus/thunderbolt/devices/"$tbt_dev" | grep KERNEL | tail -n 2 | grep -v pci0000 | cut -d "\"" -f 2)

  verify_tbt_root_pci "$ROOT_PCI"
  # For debug
  # echo "PF_BIOS:$PF_BIOS platform, ROOT_PCI:$ROOT_PCI"
}

enable_authorized()
{
  local aim_folders=""
  local aim_folder=""
  local domain=""
  local route=""

  aim_folders=$(ls -1 ${TBT_PATH} \
                | grep "$REGEX_DEVICE" \
                | grep -v ":" \
                | grep -v "0$" \
                | awk '{ print length(), $0 | "sort -n" }' \
                | cut -d ' ' -f 2)

  [ -z "$aim_folders" ] && test_print_trc "Device folder does not exist" && return 1

  for aim_folder in ${aim_folders}; do
    domain=""
    route=""
    domain=$(echo "$aim_folder" | awk -F "-" '{print $1}')
    route=$(echo "$aim_folder" | awk -F "-" '{print $2}')

    if [ -e "${TBT_PATH}/${aim_folder}/${AUTHORIZE_FILE}" ];then
      AUTHORIZE_INFO=$(cat "${TBT_PATH}/${aim_folder}/${AUTHORIZE_FILE}")
      if [ "$AUTHORIZE_INFO" == "0" ]; then
        test_print_trc "tbauth -d $domain -r $route: ${TBT_PATH}/${aim_folder}/${AUTHORIZE_FILE}"
        tbauth -d "$domain" -r "$route" || \
        test_print_trc "Change ${TBT_PATH}/${aim_folder}/${AUTHORIZE_FILE} for $aim_folder failed!"
        sleep 4
      else
        test_print_trc "${TBT_PATH}/${aim_folder}/${AUTHORIZE_FILE}: ${AUTHORIZE_INFO}"
      fi
    fi
  done
}

check_device_sysfs()
{
  REGEX_TARGET=$1
  AIM_FILE=$2
  local aim_folders=""

  test_print_trc "____________________Now Cheking '$AIM_FILE'___________________"

  aim_folders=$(ls -1 ${TBT_PATH} \
                | grep "$REGEX_TARGET" \
                | awk '{ print length(), $0 | "sort -n" }' \
                | cut -d ' ' -f 2)

  [ -z "$aim_folders" ] && test_print_trc "AIM floder does not exist" && return 1
  for aim_folder in ${aim_folders}; do
    if [ -e "${TBT_PATH}/${aim_folder}/${AIM_FILE}" ];then
      #test_print_trc "File $AIM_FILE is found on $TBT_PATH/$aim_folder"
      FILE_INFO=$(cat ${TBT_PATH}/"$aim_folder"/"$AIM_FILE")
      if [ "$FILE_INFO" != "" ]; then
        test_print_trc "${TBT_PATH}/${aim_folder}/${AIM_FILE}:   |$FILE_INFO"
      else
        test_print_trc "TBT file $TBT_PATH/${aim_folder}/${AIM_FILE} is null, should not be null"
        return 1
      fi
    else
      test_print_trc "File $AIM_FILE is not found or not a file on $TBT_PATH/$aim_folder"
      continue
    fi
  done
  return $?
}

fill_key()
{
  local source_key=$1
  local aim_folder=$2
  local key_file="key"
  local key_path="$HOME/keys"
  local home_key=""
  local verify_key=""

  cat "$key_path"/"$source_key" > "$TBT_PATH"/"$aim_folder"/"$key_file"
  home_key=$(cat "$key_path"/"$source_key")
  verify_key=$(cat "$TBT_PATH"/"$aim_folder"/"$key_file")
  test_print_trc "${key_path}/${source_key}:$home_key"
  test_print_trc "${TBT_PATH}/${aim_folder}/${key_file}:$verify_key"
}

check_security_mode()
{
  DOMAIN="domain0/security"
  [ -e ${TBT_PATH}/${DOMAIN} ] && SECURITY_RESULT=$(cat ${TBT_PATH}/${DOMAIN})
  [ -z "$SECURITY_RESULT" ] && test_print_trc "SECURITY_RESULT is null." && \
    return 1
  echo "$SECURITY_RESULT"
}

check_device_file()
{
  for DEVICE_FILE in ${DEVICE_FILES}; do
    check_device_sysfs "$REGEX_DEVICE" "$DEVICE_FILE"
  done
  test_print_trc "Check power sysfs files"

  for POWER_FILE in ${POWER_FILES}; do
    check_device_sysfs "$REGEX_DEVICE" "$POWER_FILE"
  done
}

check_domain_file()
{
  for DOMAIN_FILE in ${DOMAIN_FILES}; do
    check_device_sysfs "$REGEX_DOMAIN" "$DOMAIN_FILE"
  done
}

check_32bytes_key()
{
  local key_file=$1
  local key_content=""
  local key_len=""
  local key_number=64

  if [ -e "$key_file" ]; then
    test_print_trc "$key_file already exist"
    key_content=$(cat "$key_file")
    key_len=${#key_content}
    if [ "$key_len" -eq "$key_number" ]; then
      test_print_trc "$key_file lenth is 64, ok"
      return 0
    else
      test_print_trc "$key_file lenth is not 64:$key_content"
      test_print_trc "Key lenth wrong, regenerate $key_file"
      openssl rand -hex 32 > "$key_file"
      return 2
    fi
  else
    test_print_trc "No key file, generate $key_file"
    openssl rand -hex 32 > "$key_file"
    return 1
  fi
}

wrong_password_check()
{
  local key_file="key"
  local key_path="$HOME/keys"
  local aim_folder=$1
  local error_key="error.key"
  local key_content=""
  local key_len=""
  local compare=""
  local author_result=""
  local author_file="authorized"

  if [ -e "${TBT_PATH}/${aim_folder}/${key_file}" ];then
    if [ -e "${TBT_PATH}/${aim_folder}/${author_file}" ]; then
      author_result=$(cat ${TBT_PATH}/${aim_folder}/${author_file})
      if [ "$author_result" != "0" ]; then
        test_print_trc "${TBT_PATH}/${aim_folder}/${author_file}:$author_result"
        test_print_trc "authorized already passed, skip"
        author_result=""
        return 1
      else
        author_result=""
      fi
    fi

    check_32bytes_key "${key_path}/${error_key}"
    if [ -e "${key_path}/${aim_folder}.key" ]; then
      compare=$(diff "${key_path}/${error_key}" "${key_path}/${aim_folder}.key")
      if [ -z "$compare" ]; then
        test_print_trc "${key_path}/${error_key} the same as correct one, regenerate"
        openssl rand -hex 32 > "${key_path}/${error_key}"
      fi
    fi
    fill_key "$error_key" "$aim_folder"
    test_print_trc "fill 2 into ${TBT_PATH}/${aim_folder}/${author_file}:"
    echo 2 > "${TBT_PATH}/${aim_folder}/${author_file}"
    sleep 1
    author_result=$(cat "${TBT_PATH}/${aim_folder}/${author_file}")
    if [ "$author_result" == "0" ]; then
      test_print_trc "${TBT_PATH}/${aim_folder}/${author_file}:$author_result passed"
    else
      test_print_trc "${TBT_PATH}/${aim_folder}/${author_file}:$author_result failed"
    fi

  else
    test_print_trc "File key is not found on $TBT_PATH/$aim_folder"
  fi
}

wrong_password_test()
{
  local key_path="$HOME/keys"
  local regex_target=$REGEX_DEVICE
  local aim_folder=""
  local aim_folders=""
  local author_file="authorized"

  test_print_trc "Secure mode wrong passsword test next:"
  aim_folders=$(ls -1 ${TBT_PATH} \
                | grep "$regex_target" \
                | grep -v ":" \
                | grep -v "0$" \
                | awk '{ print length(), $0 | "sort -n" }' \
                | cut -d ' ' -f 2)

  [ -z "$aim_folders" ] && test_print_trc "aim_folders:aim_folders does not exist" && return 1
  [ -d "$key_path" ] || mkdir "$key_path"

  for aim_folder in ${aim_folders}; do
    wrong_password_check "$aim_folder"
  done
}

secure_mode_test()
{
  local key_path="$HOME/keys"
  local regex_target=$REGEX_DEVICE
  local aim_folder=""
  local domain=""
  local route=""
  local aim_folders=""
  local author_file="authorized"
  local time=3
  local author_path="/sys/bus/thunderbolt/devices/*/authorized"
  local result=""

  test_print_trc "Secure mode verify correct passsword test next:"
  aim_folders=$(ls -1 ${TBT_PATH} \
                | grep "$regex_target" \
                | grep -v ":" \
                | grep -v "0$" \
                | awk '{ print length(), $0 | "sort -n" }' \
                | cut -d ' ' -f 2)
  [ -z "$aim_folders" ] && test_print_trc "AIM floder:$aim_folders does not exist" && return 1
  [ -d "$key_path" ] || {
    rm -rf "$key_path"
    mkdir -p "$key_path"
  }
  for((i=1; i<=time; i++)); do
    result=$(grep -H . "$author_path" 2>/dev/null |awk -F ':' '{print $NF}' | grep 0)
    if [ -z "$result" ]; then
      test_print_trc "authorized all ok"
      break
    else
      test_print_trc "$i round set 2 to authorized for secure mode:"
      for aim_folder in ${aim_folders}; do
        domain=$(echo "$aim_folder" | awk -F "-" '{print $1}')
        route=$(echo "$aim_folder" | awk -F "-" '{print $2}')
        echo "tbauth -d domain -r route -A ${key_path}_${aim_folder}"
        tbauth -d domain -r route -A "${key_path}_${aim_folder}"
        echo "tbauth -d domain -r route -C ${key_path}_${aim_folder}"
        tbauth -d domain -r route -C "${key_path}_${aim_folder}"
      done
    fi
  done
  if [ "$i" -ge "$time" ]; then
    test_print_trc "[WARN] Need to check log carefully i reach $i, please check log!"
    test_print_trc "It's better unplug and plug the TBT devices and test again!"
    enable_authorized
  fi
}

# This function will view request domain and request tbt branch devices
# and will write the topo result into $TOPO_FILE
# Inuput:
#   $1: domain num, 0 for domain0, 1 for domain1
#   $2: branch num, 1 for domainX-1, 3 for domainX-3
# Return: 0 for true, otherwise false or die
topo_view()
{
  local domainx=$1
  local tn=$2
  local tbt_sys=""
  local tbt_file=""
  local dev_name="device_name"
  local device_topo=""
  local file_topo=""
  local device_num=""

  # Get tbt sys file in connection order
  tbt_sys=$(ls -l ${TBT_PATH}/"$domainx"*"$tn" 2>/dev/null \
            | grep "-" \
            | awk '{ print length(), $0 | "sort -n" }' \
            | tail -n 1 \
            | awk -F "${REGEX_DOMAIN}${domainx}/" '{print $2}' \
            | tr '/' ' ')

  [ -n "$tbt_sys" ] || {
    #echo "No tbt device in $domainx-$tn!"
    return 1
  }

  # Get last file
  last=$(echo "$tbt_sys" | awk '{print $NF}')
  device_num=$(echo "$tbt_sys" | awk '{print NF-1}')

  # Last file not add <-> in the end
  for tbt_file in ${tbt_sys}; do
    device_file=""
    if [ "$tbt_file" == "$last" ]; then
      device_file=$(cat "${TBT_PATH}/${tbt_file}/${dev_name}" 2>/dev/null)
      device_topo=${device_topo}${device_file}
      file_topo=${file_topo}${tbt_file}
    else
      device_file=$(cat "${TBT_PATH}/${tbt_file}/${dev_name}" 2>/dev/null)
      [[ -n "$device_file" ]] || device_file="no_name"
      # For alignment for such as 0-0 and device name, device name is longer
      device_file_num=${#device_file}
      tbt_file_num=${#tbt_file}
      if [[ "$device_file_num" -gt "$tbt_file_num" ]]; then
        gap=$((device_file_num - tbt_file_num))
        device_topo=${device_topo}${device_file}" <-> "
        file_topo=${file_topo}${tbt_file}
        for ((c=1; c<=gap; c++)); do
          file_topo=${file_topo}" "
        done
        file_topo=${file_topo}" <-> "
      else
        device_topo=${device_topo}${device_file}" <-> "
        file_topo=${file_topo}${tbt_file}" <-> "
      fi
    fi
  done

  echo "device_topo: $device_topo" >> $TOPO_FILE
  echo "file_topo  : $file_topo" >> $TOPO_FILE
  tblist 2>/dev/null
}

topo_name()
{
  local tbt_sys=$1
  local devs_file=$2
  local tbt_file=""
  local dev_name="device_name"
  local device_topo=""
  local file_topo=""

  [ -n "$tbt_sys" ] || {
    echo "No tbt device in tbt_sys:$tbt_sys"
    return 1
  }

  # Get last file
  last=$(echo "$tbt_sys" | awk '{print $NF}')

  # Last file not add <-> in the end
  for tbt_file in ${tbt_sys}; do
    device_file=""
    if [ "$tbt_file" == "$last" ]; then
      device_file=$(cat "${TBT_PATH}/${tbt_file}/${dev_name}" 2>/dev/null)
      device_topo=${device_topo}${device_file}
      file_topo=${file_topo}${tbt_file}
    else
      device_file=$(cat "${TBT_PATH}/${tbt_file}/${dev_name}" 2>/dev/null)
      [[ -n "$device_file" ]] || device_file="no_name"
      # For alignment for such as 0-0 and device name, device name is longer
      device_file_num=${#device_file}
      tbt_file_num=${#tbt_file}
      if [[ "$device_file_num" -gt "$tbt_file_num" ]]; then
        gap=$((device_file_num - tbt_file_num))
        device_topo=${device_topo}${device_file}" <-> "
        file_topo=${file_topo}${tbt_file}
        for ((c=1; c<=gap; c++)); do
          file_topo=${file_topo}" "
        done
        file_topo=${file_topo}" <-> "
      else
        device_topo=${device_topo}${device_file}" <-> "
        file_topo=${file_topo}${tbt_file}" <-> "
      fi
    fi
  done
  echo "device_topo: $device_topo"
  echo "device_topo: $device_topo" >> "$devs_file"
  echo "file_topo  : $file_topo"
  echo "file_topo  : $file_topo" >> "$devs_file"
}

usb4_view()
{
  local domainx=$1
  local tn=$2
  local tbt_sys_file="/tmp/tbt_sys.txt"
  local tbt_devs=""
  local device_num=""
  local dev_item=""
  local check_point=""

  ls -l "$TBT_PATH"/"$domainx"*"$tn" 2>/dev/null \
    | grep "-" \
    | awk -F "${REGEX_DOMAIN}${domainx}/" '{print $2}' \
    | awk '{ print length(), $0 | "sort -n" }' \
    | grep -v ":" \
    | grep -v "_" \
    | cut -d ' ' -f 2 \
    | tr '/' ' ' \
    > $tbt_sys_file
  # need tbt devices in order
  tbt_devs=$(ls ${TBT_PATH} 2>/dev/null \
    | grep "-" \
    | grep -v ":" \
    | grep "^${domainx}" \
    | grep "${tn}$" \
    | awk '{ print length(), $0 | "sort -n" }' \
    | cut -d ' ' -f 2)
  device_num=$(ls ${TBT_PATH} \
    | grep "^${domainx}" \
    | grep -v ":" \
    | grep "${tn}$" \
    | wc -l)
  echo "$domainx-$tn contains $device_num tbt devices."
  echo "$domainx-$tn contains $device_num tbt devices." >> $TOPO_FILE
  cat /dev/null > "${DEV_FILE}_${domainx}_${tn}"
  cp -rf "$tbt_sys_file" "${DEV_FILE}_${domainx}_${tn}"
  for tbt_dev in $tbt_devs; do
    dev_item=""
    dev_item=$(cat "$tbt_sys_file" | grep "${tbt_dev}$")
    [[ -z "$dev_item" ]] && {
      echo "WARN:dev_item is null for tbt_dev:$tbt_dev"
      continue
    }
    check_point=$(cat "$tbt_sys_file" \
      | grep -v "${dev_item}$" \
      | grep "${dev_item}" \
      | head -n 1)
    [[ -z "$check_point" ]] && {
      #echo "check_point for ${dev_item} is null"
      continue
    }
    sed -i "/${check_point}$/d" "${DEV_FILE}_${domainx}_${tn}"
    sed -i "s/${dev_item}$/${check_point}/g" "${DEV_FILE}_${domainx}_${tn}"
  done
  while IFS= read -r line
  do
    topo_name "$line" "$TOPO_FILE"
  done < "${DEV_FILE}_${domainx}_${tn}"
}

tbt_dev_name()
{
  local domainx=$1
  local tn=$2
  local dev=""
  local tbt_devs=""
  local tbt_dev=""
  local dev_name="device_name"
  local cp=""

  cat /dev/null > "${DEV_LIST}_${domainx}_${tn}"
  while IFS= read -r line
  do
    for dev in $line; do
      cp=""
      cp=$(cat ${DEV_LIST}_"$domainx"_"$tn" | grep "$dev")
      [[ -z "$cp" ]] || continue
      [[ "$dev" == *"-0" ]] && continue
      echo "$dev" >> "${DEV_LIST}_${domainx}_${tn}"
    done
  done < "${DEV_FILE}_${domainx}_${tn}"

  # Get tbt dev file in connection order
  tbt_devs=""
  tbt_devs=$(cat "$DEV_LIST"_"$domainx"_"$tn")

  for tbt_dev in $tbt_devs; do
    echo "$tbt_dev" >> "$TBT_DEV_FILE"
  done
}

# This function will check how many tbt device connected and
# show the tbt devices how to connect, which one connect with which one
# Inuput: NA
# Return: 0 for true, otherwise false or die
topo_tbt_show()
{
  # tbt spec design tbt each domain will seprate to like 0-1 or 0-3 branch
  local t1="1"
  local t3="3"
  local domains=""
  local domain=""
  local topo_result=""

  # domains example 0  1
  domains=$(ls $TBT_PATH/ \
            | grep "$REGEX_DOMAIN" \
            | grep -v ":" \
            | awk -F "$REGEX_DOMAIN" '{print $2}' \
            | awk -F "->" '{print $1}')
  cat /dev/null > "$TBT_DEV_FILE"
  cat /dev/null > $TOPO_FILE

  for domain in ${domains}; do
    #topo_view "$domain" "$t1"
    usb4_view "$domain" "$t1"
    #topo_view "$domain" "$t3"
    usb4_view "$domain" "$t3"
    tbt_dev_name "$domain" "$t1"
    tbt_dev_name "$domain" "$t3"
  done
  topo_result=$(cat $TOPO_FILE)
  [[ -n "$topo_result" ]] || {
    echo "tbt $TOPO_FILE is null:$topo_result!!!"
    exit 2
  }
}

only_show_topo() {
  # tbt spec design tbt each domain will seprate to like 0-1 or 0-3 branch
  local t1="1"
  local t3="3"
  local domains=""
  local domain=""

  # domains example 0  1
  domains=$(ls $TBT_PATH/ \
            | grep "$REGEX_DOMAIN" \
            | grep -v ":" \
            | awk -F "$REGEX_DOMAIN" '{print $2}' \
            | awk -F "->" '{print $1}')
  cat /dev/null > "$TBT_DEV_FILE"
  cat /dev/null > $TOPO_FILE

  for domain in ${domains}; do
    #topo_view "$domain" "$t1"
    usb4_view "$domain" "$t1"
    #topo_view "$domain" "$t3"
    usb4_view "$domain" "$t3"
  done
}

authorize_show_tbt_devices()
{
  SECURITY=$(check_security_mode)
  if [ "$SECURITY" == "user" ]; then
    for ((i = 1; i <= 10; i++))
    do
      CHECK_RESULT=$(cat ${TBT_PATH}/*/${AUTHORIZE_FILE} | grep 0)
      if [ -z "$CHECK_RESULT" ]; then
        test_print_trc "All authorized set to 1"
        break
      else
        test_print_trc "$i round to enable authorized"
        enable_authorized
      fi
    done
  elif [ "$SECURITY" == "none" ]; then
    test_print_trc "Security Mode: $SECURITY"
    #check_authorized
  elif [ "$SECURITY" == "secure" ]; then
    test_print_trc "Security Mode: $SECURITY"
    wrong_password_test
    secure_mode_test
  elif [ "$SECURITY" == "dponly" ]; then
    test_print_trc "Security Mode: $SECURITY"
  else
    test_print_trc "Get wrong mode: $SECURITY"
    return 1
  fi

  if [[ -n "$VERBOSE" ]]; then
    check_device_file
    check_domain_file
  fi
  echo "tblist:"
  tblist
  echo "tbt topo:"
  topo_tbt_show
}

check_usb_type()
{
  local dev_node=$1
  local speed=""

  speed=$(udevadm info --attribute-walk --name="$dev_node" \
        | grep speed \
        | head -n 1 \
        | cut -d '"' -f 2)

  case $speed in
    480)
      DEV_TYPE="USB2.0"
      ;;
    5000)
      DEV_TYPE="USB3.0"
      ;;
    10000)
      DEV_TYPE="USB3.1"
      ;;
    *)
      echo "WARN:$dev_node:USB unknow speed->$speed"
      DEV_TYPE="USB_unknow_type"
      ;;
  esac
}

stuff_in_tbt()
{
  local dev_node=$1
  local dev_pci_h=""
  local dev_pci_d=""
  local tbt_pci=""
  local num=""
  local num_add=""

  dev_pci_h=$(udevadm info --attribute-walk --name="$dev_node" \
          | grep "looking" \
          | head -n 1 \
          | awk -F "0000:" '{print $NF}' \
          | cut -d ':' -f 1)
  dev_pci_d=$((0x"$dev_pci_h"))
  for ((num=1;num<=TBT_NUM;num++)); do
    TBT_DEV_NAME=""
    DEV_PCI=""
    num_add=$((num+1))

    [[ "$num_add" -gt "$TBT_NUM" ]] && {
      TBT_DEV_NAME=$(sed -n ${num}p $TBT_DEV_FILE)
      DEV_PCI=$dev_pci_h
      break
    }

    tbt_pci=$(sed -n ${num_add}p $PCI_DEC_FILE)
    if [[ "$dev_pci_d" -lt "$tbt_pci" ]]; then
      TBT_DEV_NAME=$(sed -n ${num}p $TBT_DEV_FILE)
      DEV_PCI=$dev_pci_h
      #echo "$dev_node pci:$DEV_PCI connected with $TBT_DEV_NAME"
      break
    else
      continue
    fi
  done
  [[ -n "$TBT_DEV_NAME" ]] || {
    echo " No detect $dev_node dev:$dev_pci_d us:$tbt_pci connected with which tbt device!!!"
    return 1
  }
}

dev_under_tbt()
{
  local dev_node=$1
  local dev_tp=""
  DEV_SERIAL=""
  DEV_TYPE=""

  pci_dev=$(udevadm info --attribute-walk --name="$dev_node" \
          | grep "KERNEL" \
          | tail -n 2 \
          | head -n 1 \
          | awk -F '==' '{print $NF}' \
          | cut -d '"' -f 2)
  #echo "$dev_node pci_dev:$pci_dev"
  if [[ "$pci_dev" == *"$ROOT_PCI"* ]]; then
    #echo "$dev_node is under tbt device"
    dev_tp=$(udevadm info --query=all --name="$dev_node" \
              | grep "ID_BUS=" \
              | cut -d '=' -f 2)
    DEV_SERIAL=$(udevadm info --query=all --name="$dev_node" \
              | grep "ID_SERIAL=" \
              | cut -d '-' -f 1 \
              | cut -d '=' -f 2)
    case $dev_tp in
      ata)
        DEV_TYPE="HDD"
        ;;
      usb)
        check_usb_type "$dev_node"
        ;;
      *)
        echo "WARN:$dev_node is one unknow type:$dev_tp"
        DEV_TYPE="$dev_tp"
        ;;
    esac
    stuff_in_tbt "$dev_node"
    echo " |-> $dev_node $DEV_TYPE pci-${DEV_PCI}:00 $DEV_SERIAL $TBT_DEV_NAME" >> $STUFF_FILE
    return 0
  else
    return 1
  fi
}

list_tbt_stuff()
{
  local tbt_devs=""
  local tbt_dev=""
  local tbt_stuff=""
  local tbt_de_name=""

  cat /dev/null > $TBT_STUFF_LIST
  tbt_devs=$(cat $TBT_DEV_FILE)
  for tbt_dev in $tbt_devs; do
    tbt_stuff=""
    tbt_de_name=""
    tbt_de_name=$(cat ${TBT_PATH}/${tbt_dev}/device_name)
    echo "$tbt_dev:$tbt_de_name" >> $TBT_STUFF_LIST
    tbt_stuff=$(cat $STUFF_FILE \
              | grep "${tbt_dev}$" \
              | awk -F " $tbt_dev" '{print $1}')
    [[ -z "$tbt_stuff" ]] || \
      echo "$tbt_stuff" >> $TBT_STUFF_LIST
  done
  echo "Show detailed stuff under each tbt device:"
  cat $TBT_STUFF_LIST
}

find_tbt_dev_stuff()
{
  local dev_nodes=""
  local dev_node=""

  cat /dev/null > $STUFF_FILE
  dev_nodes=$(ls -1 /dev/sd? 2>/dev/null)
  [[ -z "$dev_nodes" ]] && {
    echo "No /dev/sd? node find:$dev_nodes"
    exit 0
  }
  for dev_node in $dev_nodes; do
    dev_under_tbt "$dev_node"
    [[ "$?" -eq 0 ]] || continue
  done
  list_tbt_stuff
}

check_tbt_us_pci()
{
  local tbt_dev_num=""
  local tbt_us_num=""

  tbt_dev_num=$(cat $TBT_DEV_FILE | wc -l)
  tbt_us_num=$(cat $PCI_DEC_FILE | wc -l)

  [[ "$tbt_dev_num" -eq "$tbt_us_num" ]] || {
    echo "$TBT_DEV_FILE num:$tbt_dev_num not equal $PCI_DEC_FILE num:$tbt_us_num"
    echo "WARN: tbt stuffs maybe not correct due to above reason!!!"
    if [[ "$tbt_dev_num" -gt "$tbt_us_num" ]]; then
      TBT_NUM=$tbt_us_num
    else
      TBT_NUM=$tbt_dev_num
    fi
    echo "TBT_NUM:$TBT_NUM"
    return 1
  }
  TBT_NUM=$tbt_dev_num
}

check_tb_tools() {
  local check_tbauth=""
  local check_tblist=""

  check_tbauth=$(which "$TBAUTH")
  [[ -z "$check_tbauth" ]] && {
    test_print_trc "[ERROR] No $TBAUTH tool in $CARGO_PATH, please cargo build; cargo install --path ."
  }

  check_tblist=$(which "$TBLIST")
  [[ -z "$check_tblist" ]] && {
    test_print_trc "[ERROR] No $TBLIST in $CARGO_PATH, please cargo build; cargo install --path ."
  }
}

show_authorized_tbt_details() {
  check_tb_tools

  rm -rf /root/test_tbt_1.log

  if [[ -n "$VERBOSE" ]]; then
    pci_result=$(lspci -t)
    test_print_trc "lspci -t: $pci_result"
    check_device_file
    check_domain_file
    topo_tbt_show
  fi

  authorize_show_tbt_devices
  echo
  # due to falcon ridge find pci a little slow
  sleep 1
  find_root_pci

  cat /dev/null > $PCI_HEX_FILE
  cat /dev/null > $PCI_DEC_FILE
  # will not impact, it will scan all PCI
  tbt_us_pci "0"
  check_tbt_us_pci
  find_tbt_dev_stuff
}

while getopts ':vsh' arg; do
  case $arg in
    h)
      usage
      ;;
    v)
      VERBOSE="1"
      ;;
    s)
      only_show_topo
      exit 0
      ;;
    *)
      usage
      ;;
  esac
done

show_authorized_tbt_details
